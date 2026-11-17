// Single source for page copy that is also published as structured data
// (JSON-LD) and as plain text for LLMs (/llms.txt), so the three never drift.

export const SITE_URL = "https://awake-site.vercel.app";
export const REPO_URL = "https://github.com/nolliebigspin/awake";
const LATEST = `${REPO_URL}/releases/latest/download`;

export const name = "awake";
export const tagline = "Keep your machine awake and your status Available.";
export const description =
  "Free, open-source tray app and CLI for macOS, Windows and Linux. It stops your computer from sleeping and resets the OS idle timer, so VPNs stay connected and Teams and Slack keep showing Available. With a stop timer.";
// One plain sentence that answers "what is awake?" on its own.
export const summary =
  "awake is a free, open-source menu bar app and command-line tool for macOS, Windows and Linux that stops your computer from sleeping and keeps Microsoft Teams, Slack and similar apps from switching your status to Away.";

export const author = { name: "Alec Winter", url: "https://awinter.dev" };

export type Download = { label: string; file: string; primary?: boolean };
export type Platform = {
  name: string;
  os: string;
  requirements: string;
  downloads: Download[];
};

export const platforms: Platform[] = [
  {
    name: "macOS",
    os: "macOS 11 or later",
    requirements: "macOS 11 or later · Apple silicon and Intel",
    downloads: [{ label: ".dmg", file: "Awake-macOS.dmg", primary: true }],
  },
  {
    name: "Windows",
    os: "Windows 10, Windows 11",
    requirements: "Windows 10 or 11 · x64 · no admin rights needed",
    downloads: [
      { label: ".exe", file: "Awake-Windows-Setup.exe", primary: true },
      { label: ".msi", file: "Awake-Windows.msi" },
    ],
  },
  {
    name: "Linux",
    os: "Linux",
    requirements: "x86_64 · Ubuntu 22.04 or newer, most other distros via AppImage",
    downloads: [
      { label: ".deb", file: "Awake-Linux-amd64.deb", primary: true },
      { label: ".AppImage", file: "Awake-Linux-x86_64.AppImage" },
    ],
  },
];

export const downloadUrl = (file: string) => `${LATEST}/${file}`;

export const features = [
  {
    title: "Stays Available",
    text: "Chat apps go by how long you haven't touched the keyboard or mouse. awake resets that timer and checks that the reset really worked.",
  },
  {
    title: "Stops when you want",
    text: "Keep awake indefinitely or for 30 minutes up to 24 hours. When the time is up, awake switches off and lets you know.",
  },
  {
    title: "macOS, Windows, Linux",
    text: "A tray app for everyday use and a CLI for scripts and servers. Both keep VPNs, downloads and remote sessions alive.",
  },
];

export const cliInstall = [
  { label: "macOS & Linux · Homebrew", command: "brew install nolliebigspin/tap/awake" },
  {
    label: "macOS & Linux · install script",
    command: `curl --proto '=https' --tlsv1.2 -LsSf ${LATEST}/awake-cli-installer.sh | sh`,
  },
  {
    label: "Windows · PowerShell",
    command: `powershell -ExecutionPolicy Bypass -c "irm ${LATEST}/awake-cli-installer.ps1 | iex"`,
  },
];

// `html` may contain <code>; it is rendered as-is on the page.
export const cliCommands = [
  { command: "awake", html: "Keep awake until you press Ctrl+C." },
  {
    command: "awake --for 3h",
    html: "Stop automatically after 3 hours. Also <code>30m</code>, <code>1h30m</code> and so on.",
  },
  { command: "awake status --test", html: "Check which methods work on this machine." },
  { command: "awake install", html: "Start at login. Undo with <code>awake uninstall</code>." },
];

// Answers are HTML (links, <em>, <code>). Root-relative links are fine here;
// the JSON-LD and llms.txt builders make them absolute.
export const faq = [
  {
    question: "What does awake do?",
    answer:
      "It does two things. It stops your computer from going to sleep, and it resets the system idle timer with a tiny synthetic input (a 1&nbsp;px mouse move or an F15 key press) before chat apps notice you're idle. After every reset it checks that the idle time really dropped, and tries another method if it didn't.",
  },
  {
    question: "Why isn't preventing sleep enough to stay Available in Teams or Slack?",
    answer:
      "Teams, Slack and similar apps decide you're Away from the operating system's input idle time: how long ago the last keyboard or mouse event happened. Tools like <code>caffeinate</code> only keep the machine from sleeping, so the idle timer keeps counting and your status turns Away after about 5 minutes. awake also resets that timer.",
  },
  {
    question: "Is awake free?",
    answer: `Yes. awake is free and open source under the <a href="${REPO_URL}/blob/main/LICENSE">MIT License</a>. There is no account, subscription or upsell.`,
  },
  {
    question: "Why does macOS ask for Accessibility access?",
    answer:
      "On current macOS, the only way to reset the idle timer that chat apps read is a tiny synthetic mouse move, and posting input needs Accessibility access. Without it, awake still keeps the Mac awake, but Teams may show you as Away. The app's menu has a <em>Fix Accessibility access…</em> item that opens the right settings pane, and Awake notifies you if it ever can't keep you Available.",
  },
  {
    question: "Does it work on Linux with Wayland?",
    answer: `X11 works fully. On Wayland it depends on the desktop: KDE works, GNOME needs a one-time uinput setup. <code>awake status --test</code> tells you what works, and the <a href="${REPO_URL}#linux-tray-and-wayland-limitations">README</a> explains the details.`,
  },
  {
    question: "Can I use it on my work computer?",
    answer:
      "awake simulates keyboard and mouse input (an F15 key press or a 1&nbsp;px mouse move). Security software on managed devices may flag or block this, and it may be against your employer's IT or presence policy. Check with your IT department first. You are responsible for how you use it.",
  },
  {
    question: "Does awake collect any data?",
    answer:
      'No. The app has no account, telemetry or tracking, and only contacts GitHub when you click <em>Check for updates</em>. This website counts visits and download clicks anonymously, without cookies. See the <a href="/privacy">privacy policy</a>.',
  },
];

const decodeEntities = (s: string) =>
  s.replace(/&nbsp;/g, " ").replace(/&amp;/g, "&").replace(/&lt;/g, "<").replace(/&gt;/g, ">");

const absolute = (href: string) => (href.startsWith("/") ? SITE_URL + href : href);

/** HTML answer → plain text, for JSON-LD. */
export const toText = (html: string) => decodeEntities(html.replace(/<[^>]+>/g, ""));

/** HTML answer → Markdown, for llms.txt. Handles the few tags used above. */
export const toMarkdown = (html: string) =>
  decodeEntities(
    html
      .replace(/<a href="([^"]+)">(.*?)<\/a>/g, (_, href, text) => `[${text}](${absolute(href)})`)
      .replace(/<\/?code>/g, "`")
      .replace(/<\/?em>/g, "*")
      .replace(/<[^>]+>/g, ""),
  );
