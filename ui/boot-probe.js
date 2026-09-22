/* Boot flight recorder — runs before everything else. On success it stays
   silent: the window title is the product's ("Spruce Grove"), not an
   instrumentation readout. Only failures speak, stamping the title so a
   human (or osascript System Events) can see exactly how boot died:
     boot:ERROR <message>      → uncaught exception during boot
     boot:REJECTION <reason>   → unhandled promise rejection
   Successful boot progress is recorded by the grove_boot_marker breadcrumb
   files (invoked from main.js), which CI and scripts can read without
   polluting the packaged app's chrome. */
window.addEventListener("error", (e) => {
  const msg = String((e && e.message) || (e && e.error) || e);
  document.title = "boot:ERROR " + msg.slice(0, 110);
});

window.addEventListener("unhandledrejection", (e) => {
  const reason = String((e && e.reason) || e);
  document.title = "boot:REJECTION " + reason.slice(0, 110);
});

/* A CSP violation is a failure: a resource the app needs was blocked by
   policy (this is exactly how remote fonts and data:-images used to die
   silently in the packaged build). Stamp it like any other boot failure. */
document.addEventListener("securitypolicyviolation", (e) => {
  document.title = "boot:CSP " + String(e.violatedDirective || "unknown").slice(0, 110);
});
