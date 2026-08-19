package gg.vape.tools;

import com.sun.tools.attach.VirtualMachine;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;

public final class LinuxAttachInjector {
    private LinuxAttachInjector() {
    }

    private static void usage() {
        System.err.println("Usage: java --add-modules jdk.attach -jar "
                + "Vape421LinuxInjector.jar <pid> <agent.so> <payload.jar>");
    }

    private static Path requireRegularFile(String value, String label)
            throws Exception {
        Path path = Paths.get(value).toRealPath();
        if (!path.isAbsolute() || !Files.isRegularFile(path)
                || !Files.isReadable(path)) {
            throw new IllegalArgumentException(label
                    + " must be a readable regular file: " + value);
        }
        return path;
    }

    public static void main(String[] arguments) throws Exception {
        if (arguments.length != 3) {
            usage();
            System.exit(2);
        }
        long pid = Long.parseLong(arguments[0]);
        if (pid <= 1) {
            throw new IllegalArgumentException("Refusing invalid target PID: " + pid);
        }
        Path agent = requireRegularFile(arguments[1], "agent");
        Path payload = requireRegularFile(arguments[2], "payload");

        VirtualMachine target = VirtualMachine.attach(Long.toString(pid));
        try {
            target.loadAgentPath(agent.toString(), "payload=" + payload);
        } finally {
            target.detach();
        }
        System.out.println("Linux JVMTI agent loaded into PID " + pid);
    }
}
