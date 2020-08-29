package xyz.mendess.jukebox;

public class JukeboxLib {
    public static final String LOG_TAG = "jukebox.so";

    public static native void startUserThread(String addr);
    public static native void setRoomName(String s);
    public static native void sendCommand(String s);
}