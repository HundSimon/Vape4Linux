package gg.vape.runtime;

import java.lang.reflect.Method;

public final class NativeBridge {
    private NativeBridge() {
    }

    public static native int scb(Class<?> targetClass, byte[] bytecode);
    public static native void smd(int mode, int value);
    public static native short gks(int keyCode);
    public static native String gkn(long keyCode);
    public static native int mvk(int key, int mode);
    public static native void cpy(String text);
    public static native byte[] gcb(Class<?> targetClass);
    public static native byte[] gfb(String name);
    public static native void trs(int state);
    public static native Object inv(Method method, Object target, Object[] arguments);
    public static native String gat();
    public static native int dsv2(int fontId, String text, double x, double y,
                                   int color, float scale);
    public static native int ss_2(String value);
    public static native int mfv2(int fontId, int style, String text);
    public static native void ss(String value);
    public static native void sce(String value);

    public static boolean om(int eventId, long first, long second) {
        return false;
    }

    public static void wh(long handle) {
    }

    public static void platformSendMouse(int buttonMask, int message) {
    }

    public static short platformGetKeyState(int key) {
        return 0;
    }

    public static String platformGetKeyName(long keyData) {
        return "";
    }

    public static int platformMapKey(int code, int mapType) {
        return code;
    }

    public static void platformCopy(String text) {
    }

    public static void start() {
        System.setProperty("vape421.test.started", "true");
    }
}
