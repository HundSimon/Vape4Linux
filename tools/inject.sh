#!/usr/bin/env bash
set -euo pipefail

project_root=$(cd "$(dirname "$0")/.." && pwd)
java=/home/melaton/.local/share/FjordLauncher/java/java-runtime-delta/bin/java
injector="$project_root/build/injection/Vape421LinuxInjector.jar"
agent="$project_root/build/injection/libVape421Native.so"
payload="$project_root/build/injection/Vape421Payload.jar"

if [[ $# -ne 1 || ! $1 =~ ^[0-9]+$ || $1 -le 1 ]]; then
    echo "Usage: $0 <minecraft-java-pid>" >&2
    exit 2
fi
target_pid=$1

if [[ ! -r /proc/$target_pid/status ]]; then
    echo "Target PID does not exist or is not readable: $target_pid" >&2
    exit 1
fi
target_uid=$(awk '/^Uid:/ { print $2; exit }' "/proc/$target_pid/status")
if [[ $target_uid != $(id -u) ]]; then
    echo "Refusing a JVM owned by another user (UID $target_uid)" >&2
    exit 1
fi
target_exe=$(readlink -f "/proc/$target_pid/exe")
if [[ ${target_exe##*/} != java ]]; then
    echo "Refusing non-Java target: $target_exe" >&2
    exit 1
fi

for required in "$java" "$injector" "$agent" "$payload"; do
    if [[ ! -f $required || ! -r $required ]]; then
        echo "Required injection file is missing or unreadable: $required" >&2
        exit 1
    fi
done

echo "WARNING: this target is outside the sandbox."
echo "The injected native agent and payload will inherit all permissions of PID $target_pid."
echo "Target executable: $target_exe"
read -r -p "Type the target PID again to inject: " confirmation
if [[ $confirmation != "$target_pid" ]]; then
    echo "Injection cancelled." >&2
    exit 1
fi

exec "$java" --add-modules jdk.attach -jar "$injector" \
    "$target_pid" "$agent" "$payload"
