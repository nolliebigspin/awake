// Adds a copy button to every code block. The page works without it.
document.querySelectorAll("pre").forEach((pre) => {
  if (!navigator.clipboard) return;
  // Wrap the block so the button stays put while long commands scroll.
  const wrap = document.createElement("div");
  wrap.className = "codeblock";
  pre.replaceWith(wrap);
  wrap.appendChild(pre);

  const button = document.createElement("button");
  button.type = "button";
  button.className = "copy";
  button.textContent = "Copy";
  button.addEventListener("click", async () => {
    try {
      await navigator.clipboard.writeText(pre.innerText.trim());
      button.textContent = "Copied";
    } catch {
      button.textContent = "Copy failed";
    }
    setTimeout(() => (button.textContent = "Copy"), 1500);
  });
  wrap.appendChild(button);
});
