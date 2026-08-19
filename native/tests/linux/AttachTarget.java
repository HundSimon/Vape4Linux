package gg.vape.tests;

public final class AttachTarget {
    private AttachTarget() {
    }

    public static void main(String[] arguments) throws Exception {
        Thread renderThread = new Thread(() -> {
            while (!Thread.currentThread().isInterrupted()) {
                try {
                    Thread.sleep(100L);
                } catch (InterruptedException stopped) {
                    Thread.currentThread().interrupt();
                }
            }
        }, "Render thread");
        renderThread.setContextClassLoader(ClassLoader.getSystemClassLoader());
        renderThread.start();
        System.out.println("ATTACH_TARGET_READY");
        System.out.flush();

        long deadline = System.nanoTime() + 30_000_000_000L;
        while (!"true".equals(System.getProperty("vape421.test.started"))
                && System.nanoTime() < deadline) {
            Thread.sleep(50L);
        }
        renderThread.interrupt();
        renderThread.join();
        if (!"true".equals(System.getProperty("vape421.test.started"))) {
            throw new IllegalStateException("Agent did not start the stub payload");
        }
        System.out.println("ATTACH_AGENT_SMOKE_OK");
    }
}
