// Vercel Web Analytics: queue calls until /_vercel/insights/script.js loads.
// Kept in a file rather than inline so the CSP can stay at script-src 'self'.
window.va =
  window.va ||
  function () {
    (window.vaq = window.vaq || []).push(arguments);
  };

// Count download clicks as a custom event (Vercel Pro plan and up).
// Pro allows 2 properties per event: the file name (it already names the
// platform) and where on the page the button sits.
document.addEventListener("click", (event) => {
  const link = event.target.closest('a[href*="/releases/latest/download/"]');
  if (!link) return;
  window.va("event", {
    name: "Download",
    data: {
      file: link.pathname.split("/").pop(),
      placement: link.closest(".hero") ? "hero" : "download-section",
    },
  });
});
