# Linux native-agent port

The Linux build uses HotSpot's supported Attach API rather than remote
`ptrace`/`dlopen` injection.  Its native library exports `Agent_OnAttach`,
initializes JVMTI, adds the payload JAR to the Minecraft class-loader search,
registers the bridge methods and calls `NativeBridge.start()`.

## Build

The wrapper distribution checksum is pinned in
`gradle/wrapper/gradle-wrapper.properties`; dependency artifact hashes are
checked by `gradle/verification-metadata.xml`. After the audited cache has
been prepared, build and run the benign attach smoke test with:

```sh
./gradlew --offline --dependency-verification strict \
  prepareInjectionBundle linuxAgentSmokeTest \
  -PnativeJavaHome=/usr/lib/jvm/java-17-openjdk
```

Linux outputs are placed in `build/injection/`:

- `libVape421Native.so`
- `Vape421LinuxInjector.jar`
- `Vape421Payload.jar`
- `README.md`

## Attach

The injector, target JVM, native library and payload must be visible in the
same user, PID and mount namespaces:

```sh
java --add-modules jdk.attach \
  -jar Vape421LinuxInjector.jar \
  <pid> ./libVape421Native.so ./Vape421Payload.jar
```

The target can reject this operation when it was started with
`-XX:+DisableAttachMechanism` or `-XX:-EnableDynamicAgentLoading`.  JDK 21
also emits the standard dynamic-agent warning unless the target explicitly
allows it.

For the FjordLauncher Java runtime, list JVMs and then inject a selected PID:

```sh
/home/melaton/.local/share/FjordLauncher/java/java-runtime-delta/bin/jcmd -l
./tools/inject.sh <pid>
```

The helper requires the PID to be entered twice and deliberately does not
print the target command line, which may contain launcher credentials. The
loaded agent and payload inherit every permission held by the target JVM.

## Wayland input boundary

`PlatformInputBridge` accepts GLFW key, character, mouse, scroll and focus
callbacks and normalizes them into the client's internal input state.  The
POSIX native bridge never searches for desktop windows, subclasses a native
window procedure, or injects compositor-wide input.

Modern LWJGL3 targets still need their Minecraft/GLFW callback registration
hooked to these entry points. `GlfwWaylandInputHook` does this for the tested
Minecraft 1.21.11 Fabric intermediary runtime on its Render thread. It chains
the original key, character, mouse-button, cursor, scroll and focus callbacks
before updating the client input state. LWJGL2 Minecraft versions remain
XWayland-only; the injected agent cannot turn an X11 game window into a native
Wayland surface.
