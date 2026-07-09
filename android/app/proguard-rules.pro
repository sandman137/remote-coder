# R8/ProGuard keep rules for the release build.

# --- JNA (UniFFI bindings call native through JNA proxies + Structures) ---
-keep class com.sun.jna.** { *; }
-keepclassmembers class com.sun.jna.** { *; }
-keep class * implements com.sun.jna.** { *; }
# JNA Structures map fields by reflection — never rename/strip their members.
-keepclassmembers class * extends com.sun.jna.Structure { *; }

# --- UniFFI generated bindings (native fn interface, records, callbacks) ---
-keep class uniffi.** { *; }
-keepclassmembers class uniffi.** { *; }
-keep interface uniffi.** { *; }

# App classes referenced across the FFI callback boundary.
-keep class com.remotecoder.** { *; }

# --- Kotlin coroutines / metadata ---
-keepclassmembers class kotlinx.coroutines.** { volatile <fields>; }
-dontwarn kotlinx.coroutines.**

# --- CameraX + ML Kit barcode (reflection / native model loading) ---
-keep class androidx.camera.** { *; }
-dontwarn androidx.camera.**
-keep class com.google.mlkit.** { *; }
-keep class com.google.android.gms.** { *; }
-dontwarn com.google.mlkit.**
-dontwarn com.google.android.gms.**

# --- Firebase messaging ---
-keep class com.google.firebase.** { *; }
-dontwarn com.google.firebase.**

# Keep any JNI native method signatures intact.
-keepclasseswithmembernames class * { native <methods>; }

# --- Gson references java.awt.* in optional serializers never loaded on
# Android; suppress the R8 missing-class warnings (from missing_rules.txt).
-dontwarn java.awt.**
-dontwarn javax.annotation.**
-keep class com.google.gson.** { *; }
-keep class * extends com.google.gson.TypeAdapter { *; }
# Gson uses reflection over model fields — keep the pairing payload model.
-keepclassmembers class com.remotecoder.** {
    @com.google.gson.annotations.SerializedName <fields>;
    <fields>;
}
