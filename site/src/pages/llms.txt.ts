// /llms.txt (https://llmstxt.org): the site's content as Markdown for LLMs
// and answer engines, built from the same data as the homepage.
import type { APIRoute } from "astro";
import {
  REPO_URL,
  SITE_URL,
  cliCommands,
  cliInstall,
  downloadUrl,
  faq,
  features,
  platforms,
  summary,
  toMarkdown,
} from "../data/site";

export const GET: APIRoute = () => {
  const lines = [
    "# awake",
    "",
    `> ${summary}`,
    "",
    "awake inhibits system sleep and resets the operating system's input idle timer with a tiny synthetic input (a 1 px mouse move or an F15 key press), then checks that the reset worked. Chat apps decide you are Away from that idle timer, so tools that only prevent sleep are not enough to stay Available. It is free and open source (MIT), with no account and no telemetry. It comes as a tray app and as the `awake` CLI.",
    "",
    "## Features",
    "",
    ...features.map((f) => `- **${f.title}**: ${f.text}`),
    "",
    "## Download the tray app",
    "",
    ...platforms.map(
      (p) =>
        `- ${p.name} (${p.requirements}): ${p.downloads.map((d) => `[${d.file}](${downloadUrl(d.file)})`).join(", ")}`,
    ),
    "- macOS with Homebrew: `brew install --cask nolliebigspin/tap/awake-tray`",
    `- All files and checksums: [latest release](${REPO_URL}/releases/latest)`,
    "",
    "## Install the CLI",
    "",
    ...cliInstall.map((c) => `- ${c.label}: \`${c.command}\``),
    "",
    "## Use the CLI",
    "",
    ...cliCommands.map((c) => `- \`${c.command}\`: ${toMarkdown(c.html)}`),
    "",
    "## FAQ",
    "",
    ...faq.flatMap((item) => [`### ${item.question}`, "", toMarkdown(item.answer), ""]),
    "## Links",
    "",
    `- [Website](${SITE_URL}/)`,
    `- [README with full documentation](${REPO_URL}#readme): how it works per OS, all CLI flags, macOS Accessibility, Linux and Wayland, corporate machines`,
    `- [Source code on GitHub](${REPO_URL})`,
    `- [Privacy policy](${SITE_URL}/privacy)`,
    "",
  ];
  return new Response(lines.join("\n"), {
    headers: { "Content-Type": "text/plain; charset=utf-8" },
  });
};
