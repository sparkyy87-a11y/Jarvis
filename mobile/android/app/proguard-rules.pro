# UniFFI-Anbindung und JNA arbeiten per Reflexion – nicht umbenennen oder entfernen.
-keep class com.sun.jna.** { *; }
-keep class * implements com.sun.jna.** { *; }
-keep class uniffi.** { *; }
-dontwarn java.awt.**
