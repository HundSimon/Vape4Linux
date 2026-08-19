package gg.vape.input;

import gg.vape.runtime.NativeBridge;

import java.lang.reflect.InvocationHandler;
import java.lang.reflect.Method;
import java.lang.reflect.Proxy;
import java.util.ArrayList;
import java.util.List;
import java.util.concurrent.Executor;

/**
 * Installs process-local GLFW callbacks without depending on LWJGL3 at build
 * time. Minecraft keeps ownership of its original callbacks; each wrapper
 * first offers an event to the recovered client and delegates unconsumed
 * events to Minecraft.
 */
public final class GlfwWaylandInputHook {
    private static final List<Object> CALLBACK_REFERENCES = new ArrayList<Object>();
    private static boolean scheduled;
    private static boolean installed;

    private GlfwWaylandInputHook() {
    }

    public static synchronized void installAsync() {
        if (scheduled || !isLinux()) {
            return;
        }
        scheduled = true;
        try {
            ClassLoader loader = NativeBridge.class.getClassLoader();
            Class<?> minecraftClass = loadFirst(loader,
                    "net.minecraft.class_310", "net.minecraft.client.Minecraft");
            Object minecraft = invokeNoArgs(null, minecraftClass,
                    "method_1551", "getInstance");
            if (!(minecraft instanceof Executor)) {
                throw new IllegalStateException("Minecraft does not implement Executor");
            }
            final Object client = minecraft;
            ((Executor)minecraft).execute(new Runnable() {
                @Override
                public void run() {
                    installOnRenderThread(client);
                }
            });
        }
        catch (Throwable error) {
            scheduled = false;
            logFailure("schedule GLFW input hook", error);
        }
    }

    private static synchronized void installOnRenderThread(Object minecraft) {
        if (installed) {
            return;
        }
        try {
            Object window = invokeNoArgs(minecraft, minecraft.getClass(),
                    "method_22683", "getWindow");
            long handle = ((Number)invokeNoArgs(window, window.getClass(),
                    // 1.21.11 renamed Window#getWindow to Window#handle.
                    // Keep intermediary, SRG and older named spellings for
                    // the other supported runtime namespaces.
                    "method_4490", "m_417542_", "getWindow", "handle")).longValue();
            if (handle == 0L) {
                throw new IllegalStateException("Minecraft returned a null GLFW window");
            }

            ClassLoader loader = minecraft.getClass().getClassLoader();
            Class<?> glfw = Class.forName("org.lwjgl.glfw.GLFW", true, loader);
            install(glfw, loader, handle, "Key", new EventSink() {
                @Override
                public boolean accept(Object[] arguments) {
                    return PlatformInputBridge.onGlfwKey(intArg(arguments, 1),
                            intArg(arguments, 2), intArg(arguments, 3), intArg(arguments, 4));
                }
            });
            install(glfw, loader, handle, "CharMods", new EventSink() {
                @Override
                public boolean accept(Object[] arguments) {
                    return PlatformInputBridge.onCodePoint(intArg(arguments, 1));
                }
            });
            install(glfw, loader, handle, "MouseButton", new EventSink() {
                @Override
                public boolean accept(Object[] arguments) {
                    return PlatformInputBridge.onMouseButton(intArg(arguments, 1),
                            intArg(arguments, 2), intArg(arguments, 3));
                }
            });
            install(glfw, loader, handle, "CursorPos", new EventSink() {
                @Override
                public boolean accept(Object[] arguments) {
                    return PlatformInputBridge.onCursorPosition(doubleArg(arguments, 1),
                            doubleArg(arguments, 2));
                }
            });
            install(glfw, loader, handle, "Scroll", new EventSink() {
                @Override
                public boolean accept(Object[] arguments) {
                    return PlatformInputBridge.onScroll(doubleArg(arguments, 1),
                            doubleArg(arguments, 2));
                }
            });
            install(glfw, loader, handle, "WindowFocus", new EventSink() {
                @Override
                public boolean accept(Object[] arguments) {
                    PlatformInputBridge.onFocus(((Boolean)arguments[1]).booleanValue());
                    return false;
                }
            });

            Method getPlatform = glfw.getMethod("glfwGetPlatform");
            int platform = ((Number)getPlatform.invoke(null)).intValue();
            installed = true;
            NativeBridge.sce("GLFW input hook active; platform=" + platformName(glfw, platform));
        }
        catch (Throwable error) {
            scheduled = false;
            logFailure("install GLFW input hook", error);
        }
    }

    private static void install(Class<?> glfw, ClassLoader loader, long handle,
                                String callbackName, final EventSink sink) throws Exception {
        final Class<?> callbackInterface = Class.forName(
                "org.lwjgl.glfw.GLFW" + callbackName + "CallbackI", true, loader);
        final Object[] previous = new Object[1];
        final boolean[] clientFailureLogged = new boolean[1];
        Object wrapper = Proxy.newProxyInstance(loader, new Class<?>[]{callbackInterface},
                new InvocationHandler() {
                    @Override
                    public Object invoke(Object proxy, Method method, Object[] arguments)
                            throws Throwable {
                        if (method.isDefault()) {
                            return invokeDefault(proxy, method, arguments);
                        }
                        if ("invoke".equals(method.getName())) {
                            boolean consumed = false;
                            try {
                                consumed = sink.accept(arguments);
                            }
                            catch (Throwable error) {
                                // No recovered-client failure may cross a native
                                // GLFW callback boundary and terminate Minecraft.
                                if (!clientFailureLogged[0]) {
                                    clientFailureLogged[0] = true;
                                    logFailure("handle " + callbackInterface.getSimpleName(), error);
                                }
                            }
                            if (!consumed && previous[0] != null) {
                                try {
                                    method.invoke(previous[0], arguments);
                                }
                                catch (java.lang.reflect.InvocationTargetException error) {
                                    throw error.getCause();
                                }
                            }
                            return null;
                        }
                        if ("toString".equals(method.getName())) {
                            return "Vape GLFW " + callbackInterface.getSimpleName();
                        }
                        if ("hashCode".equals(method.getName())) {
                            return Integer.valueOf(System.identityHashCode(proxy));
                        }
                        if ("equals".equals(method.getName())) {
                            return Boolean.valueOf(proxy == arguments[0]);
                        }
                        throw new UnsupportedOperationException(method.toString());
                    }
                });
        Method setter = glfw.getMethod("glfwSet" + callbackName + "Callback",
                Long.TYPE, callbackInterface);
        previous[0] = setter.invoke(null, Long.valueOf(handle), wrapper);
        CALLBACK_REFERENCES.add(wrapper);
        CALLBACK_REFERENCES.add(previous[0]);
    }

    private static Object invokeDefault(Object proxy, Method method, Object[] arguments)
            throws Throwable {
        // InvocationHandler.invokeDefault was added after Java 8.  Reflection
        // keeps the payload Java-8 loadable while modern Minecraft runs it on
        // JDK 21, where LWJGL CallbackI default methods are available.
        Method invokeDefault = InvocationHandler.class.getMethod("invokeDefault",
                Object.class, Method.class, Object[].class);
        try {
            return invokeDefault.invoke(null, proxy, method,
                    arguments == null ? new Object[0] : arguments);
        }
        catch (java.lang.reflect.InvocationTargetException error) {
            throw error.getCause();
        }
    }

    private static Object invokeNoArgs(Object receiver, Class<?> type, String... names)
            throws Exception {
        for (String name : names) {
            for (Class<?> owner = type; owner != null; owner = owner.getSuperclass()) {
                try {
                    Method method = owner.getDeclaredMethod(name);
                    method.setAccessible(true);
                    return method.invoke(receiver);
                }
                catch (NoSuchMethodException ignored) {
                }
            }
        }
        throw new NoSuchMethodException(type.getName() + " "
                + java.util.Arrays.toString(names));
    }

    private static Class<?> loadFirst(ClassLoader loader, String... names)
            throws ClassNotFoundException {
        ClassNotFoundException failure = null;
        for (String name : names) {
            try {
                return Class.forName(name, false, loader);
            }
            catch (ClassNotFoundException error) {
                failure = error;
            }
        }
        throw failure;
    }

    private static int intArg(Object[] arguments, int index) {
        return ((Number)arguments[index]).intValue();
    }

    private static double doubleArg(Object[] arguments, int index) {
        return ((Number)arguments[index]).doubleValue();
    }

    private static String platformName(Class<?> glfw, int value) {
        String[] names = {"WAYLAND", "X11", "WIN32", "COCOA", "NULL"};
        for (String name : names) {
            try {
                if (glfw.getField("GLFW_PLATFORM_" + name).getInt(null) == value) {
                    return name.toLowerCase();
                }
            }
            catch (ReflectiveOperationException ignored) {
            }
        }
        return Integer.toString(value);
    }

    private static boolean isLinux() {
        return System.getProperty("os.name", "").toLowerCase().contains("linux");
    }

    private static void logFailure(String operation, Throwable error) {
        Throwable cause = error;
        if (error instanceof java.lang.reflect.InvocationTargetException
                && ((java.lang.reflect.InvocationTargetException)error).getCause() != null) {
            cause = ((java.lang.reflect.InvocationTargetException)error).getCause();
        }
        NativeBridge.sce("WARN " + operation + ": " + cause.getClass().getName()
                + ": " + cause.getMessage());
    }

    private interface EventSink {
        boolean accept(Object[] arguments);
    }
}
