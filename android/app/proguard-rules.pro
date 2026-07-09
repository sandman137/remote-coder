# JNA + UniFFI bindings use JNI/reflection; keep them intact.
-keep class com.sun.jna.** { *; }
-keep class uniffi.** { *; }
-keepclassmembers class uniffi.** { *; }
# Firebase messaging entry points.
-keep class com.remotecoder.notify.** { *; }
