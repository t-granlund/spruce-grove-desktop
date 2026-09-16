/* Boot flight recorder — runs before everything else, narrates into the
   window title so scripts (and humans) can see exactly how far boot got:
     boot:script-loaded tauri=object   → bridge present, scripts runnable
     boot:ERROR <message>              → uncaught exception during boot
     boot:REJECTION <reason>           → unhandled promise rejection
   Read via: osascript System Events (window title). Temporary instrument
   while the packaged-app boot chain is being debugged. */
document.title = "boot:script-loaded tauri=" + (typeof window.__TAURI__);

window.addEventListener("error", (e) => {
  const msg = String((e && e.message) || (e && e.error) || e);
  document.title = "boot:ERROR " + msg.slice(0, 110);
});

window.addEventListener("unhandledrejection", (e) => {
  const reason = String((e && e.reason) || e);
  document.title = "boot:REJECTION " + reason.slice(0, 110);
});
