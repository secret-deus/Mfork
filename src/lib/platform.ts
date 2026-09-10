// Platform flags for the shared UI. The Tauri Android WebView reports an
// Android user agent; desktop WebViews never do.
export const IS_ANDROID = /Android/i.test(navigator.userAgent);
export const IS_MOBILE = IS_ANDROID || /iPhone|iPad/.test(navigator.userAgent);
export const IS_MAC = !IS_MOBILE && /Mac/.test(navigator.userAgent);
export const IS_DESKTOP = !IS_MOBILE;
