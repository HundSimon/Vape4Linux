package gg.vape.lifecycle;

import java.io.File;

public class ClientDirectoryCleanupCallback
implements ClientLifecycleCallback {
    @Override
    public void log(String message) {
    }

    public ClientDirectoryCleanupCallback() {
        String appDataDirectory = System.getenv("APPDATA");
        // This cleanup is for the legacy Windows cache only.  On Unix,
        // concatenating a missing APPDATA value produced the relative path
        // "null/.vapeclient", which is both incorrect and unsafe to delete.
        if (appDataDirectory == null || appDataDirectory.trim().isEmpty()) {
            return;
        }
        String clientDirectoryPath = appDataDirectory + File.separator + ".vapeclient";
        File clientDirectory = new File(clientDirectoryPath);
        if (clientDirectory.exists()) {
            File[] children = clientDirectory.listFiles();
            if (children == null) {
                return;
            }
            for (File child : children) {
                if (child.getName().equals("cache")) continue;
                child.delete();
            }
        }
    }


    @Override
    public void close() {
    }
}
