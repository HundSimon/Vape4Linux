#!/bin/sh
set -eu

workspace=${1:-/workspace}
test_build="$workspace/build/linux-agent-smoke"
target_classes="$test_build/target-classes"
payload_classes="$test_build/payload-classes"
target_log="$test_build/target.log"
payload_jar="$test_build/stub-payload.jar"
agent="$workspace/build/injection/libVape421Native.so"
injector="$workspace/build/injection/Vape421LinuxInjector.jar"
java_home=${JAVA_HOME:?JAVA_HOME must name a JDK with java, javac and jar}
java="$java_home/bin/java"
javac="$java_home/bin/javac"
jar="$java_home/bin/jar"

mkdir -p "$target_classes" "$payload_classes"
"$javac" --release 11 -d "$target_classes" \
    "$workspace/native/tests/linux/AttachTarget.java"
"$javac" --release 8 -d "$payload_classes" \
    "$workspace/native/tests/linux/payload/gg/vape/runtime/NativeBridge.java" \
    "$workspace/native/tests/linux/payload/gg/vape/event/impl/EventRenderWorldPassExecutorDrain.java"
"$jar" --create --file "$payload_jar" -C "$payload_classes" .

"$java" -cp "$target_classes" gg.vape.tests.AttachTarget >"$target_log" 2>&1 &
target_pid=$!
cleanup() {
    if kill -0 "$target_pid" 2>/dev/null; then
        kill "$target_pid" 2>/dev/null || true
    fi
}
trap cleanup EXIT INT TERM

attempt=0
while ! grep -q ATTACH_TARGET_READY "$target_log" 2>/dev/null; do
    if ! kill -0 "$target_pid" 2>/dev/null; then
        cat "$target_log"
        exit 1
    fi
    attempt=$((attempt + 1))
    if [ "$attempt" -ge 100 ]; then
        echo "Attach target did not become ready" >&2
        exit 1
    fi
    sleep 0.05
done

"$java" --add-modules jdk.attach -jar "$injector" \
    "$target_pid" "$agent" "$payload_jar"
wait "$target_pid"
trap - EXIT INT TERM
grep -q ATTACH_AGENT_SMOKE_OK "$target_log"
cat "$target_log"
