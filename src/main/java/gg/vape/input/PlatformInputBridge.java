package gg.vape.input;

import java.util.Collections;
import java.util.HashMap;
import java.util.Map;

/**
 * Platform-neutral entry points for input captured inside the game process.
 *
 * <p>Linux/Wayland integrations must call these methods from the game's own
 * LWJGL or Minecraft callbacks.  They deliberately do not access compositor
 * globals or synthesize desktop-wide input.</p>
 */
public final class PlatformInputBridge {
    public static final int RELEASE = 0;
    public static final int PRESS = 1;
    public static final int REPEAT = 2;

    private static final Map<Integer, Integer> GLFW_TO_VIRTUAL_KEY;

    static {
        Map<Integer, Integer> keys = new HashMap<Integer, Integer>();
        keys.put(256, 27);  // escape
        keys.put(257, 13);  // enter
        keys.put(258, 9);   // tab
        keys.put(259, 8);   // backspace
        keys.put(260, 45);  // insert
        keys.put(261, 46);  // delete
        keys.put(262, 39);  // right
        keys.put(263, 37);  // left
        keys.put(264, 40);  // down
        keys.put(265, 38);  // up
        keys.put(266, 33);  // page up
        keys.put(267, 34);  // page down
        keys.put(268, 36);  // home
        keys.put(269, 35);  // end
        keys.put(280, 20);  // caps lock
        keys.put(281, 145); // scroll lock
        keys.put(282, 144); // num lock
        keys.put(283, 44);  // print screen
        keys.put(284, 19);  // pause
        for (int key = 290; key <= 313; ++key) {
            keys.put(key, 112 + key - 290); // F1 through F24
        }
        for (int key = 320; key <= 329; ++key) {
            keys.put(key, 96 + key - 320); // keypad 0 through 9
        }
        keys.put(330, 110); // keypad decimal
        keys.put(331, 111); // keypad divide
        keys.put(332, 106); // keypad multiply
        keys.put(333, 109); // keypad subtract
        keys.put(334, 107); // keypad add
        keys.put(335, 13);  // keypad enter
        keys.put(340, 160); // left shift
        keys.put(341, 162); // left control
        keys.put(342, 164); // left alt
        keys.put(343, 91);  // left super
        keys.put(344, 161); // right shift
        keys.put(345, 163); // right control
        keys.put(346, 165); // right alt
        keys.put(347, 92);  // right super
        keys.put(348, 93);  // menu
        GLFW_TO_VIRTUAL_KEY = Collections.unmodifiableMap(keys);
    }

    private PlatformInputBridge() {
    }

    public static int glfwToVirtualKey(int key) {
        if ((key >= 32 && key <= 96) || (key >= 48 && key <= 90)) {
            return key;
        }
        Integer mapped = GLFW_TO_VIRTUAL_KEY.get(key);
        return mapped == null ? 0 : mapped;
    }

    public static boolean onGlfwKey(int key, int scanCode, int action, int modifiers) {
        int virtualKey = glfwToVirtualKey(key);
        if (virtualKey == 0) {
            return false;
        }
        long metadata = ((long)scanCode & 0xffL) << 16;
        if (virtualKey == 163 || virtualKey == 165 || virtualKey == 161) {
            metadata |= 0x1000000L;
        }
        int event = action == RELEASE ? 257 : action == REPEAT ? 260 : 256;
        return InputEventDispatcher.getInstance().dispatch(event, virtualKey, metadata);
    }

    public static boolean onCodePoint(int codePoint) {
        if (!Character.isValidCodePoint(codePoint)) {
            return false;
        }
        if (codePoint <= Character.MAX_VALUE) {
            return InputEventDispatcher.getInstance().dispatch(258, codePoint, 0L);
        }
        char[] surrogatePair = Character.toChars(codePoint);
        boolean first = InputEventDispatcher.getInstance().dispatch(
                258, surrogatePair[0], 0L);
        boolean second = InputEventDispatcher.getInstance().dispatch(
                258, surrogatePair[1], 0L);
        return first || second;
    }

    public static boolean onMouseButton(int button, int action, int modifiers) {
        return InputEventDispatcher.getInstance().getMouseState()
                .setButtonState(button, action != RELEASE);
    }

    public static boolean onCursorPosition(double x, double y) {
        return InputEventDispatcher.getInstance().getMouseState()
                .updateCursorPosition((int)Math.round(x), (int)Math.round(y));
    }

    public static boolean onScroll(double horizontal, double vertical) {
        return InputEventDispatcher.getInstance().getMouseState()
                .setScrollDelta((int)Math.round(vertical * 120.0));
    }

    public static void onFocus(boolean focused) {
        InputFocusState state = InputEventDispatcher.getInstance().getFocusState();
        if (focused) {
            state.markFocused();
        } else {
            state.markUnfocused();
        }
    }

    public static short getVirtualKeyState(int virtualKey) {
        boolean down = InputEventDispatcher.getInstance().getKeyboardState()
                .isKeyDown(virtualKey);
        if (!down && virtualKey >= 1 && virtualKey <= 6) {
            down = InputEventDispatcher.getInstance().getMouseState()
                    .isButtonDown(virtualKey - 1);
        }
        return down ? (short)0x100 : 0;
    }
}
