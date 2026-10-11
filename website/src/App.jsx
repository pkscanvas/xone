import { useMemo, useState } from "react";
import {
  ArrowDown,
  ArrowUpRight,
  Brain,
  Cloud,
  Cpu,
  DownloadSimple,
  GithubLogo,
  HardDrives,
  List,
  LockKey,
  Robot,
  Sparkle,
  SlidersHorizontal,
  Stack,
  Newspaper,
  CursorClick,
  Keyboard,
  X,
} from "@phosphor-icons/react";

const providers = [
  { name: "Ollama", detail: "Local", icon: HardDrives },
  { name: "LM Studio", detail: "Local", icon: Cpu },
  { name: "OpenAI", detail: "Cloud", icon: Sparkle },
  { name: "Claude", detail: "Anthropic", icon: Brain },
  { name: "Groq", detail: "Cloud", icon: Cloud },
  { name: "OpenRouter", detail: "Cloud", icon: Robot },
  { name: "Custom server", detail: "OpenAI-compatible", icon: LockKey },
];

const features = [
  { icon: Sparkle, title: "A sharper Signal brief.", body: "Turn a rough idea into clear include-and-exclude criteria with AI. Then get source recommendations that fit your brief." },
  { icon: SlidersHorizontal, title: "Your focus, on a dial.", body: "Broad includes low matches and above. Focused keeps medium and high. Laser-focused shows high only. Switch instantly, without another AI call." },
  { icon: Newspaper, title: "The past day, distilled.", body: "TL:DR brings the past 24 hours into a short briefing for each Xone, with citations that open the original reporting." },
  { icon: Stack, title: "One story. More perspectives.", body: "Similar reports stay together in one article row. Expand the report stack to explore other headlines and sources." },
  { icon: CursorClick, title: "Learns from what you read.", body: "Every ten distinct article opens refine a secondary behavior profile. It guides future filtering while your Signal brief stays in charge." },
  { icon: Keyboard, title: "A calmer place to catch up.", body: "Today and All Unread collect the stories that pass your Xones’ filters. Read in light or dark mode, with adjustable type and keyboard shortcuts." },
];

const previews = [
  { label: "Light", image: "reader-light-current.png", alt: "Xone light-mode reader with Focused filtering and categorical match labels" },
  { label: "Dark", image: "reader-dark-current.png", alt: "Xone dark-mode reader with Smart Feeds and a focused article list" },
  { label: "TL:DR", image: "tldr-current.png", alt: "TL:DR briefings grouped by Xone with links to original reporting" },
];

const videos = [
  { title: "The Xone film", detail: "Latest · 0:50", src: "xone-narrative-50s-v4.mp4", poster: "xone-narrative-poster.jpg" },
  { title: "Product demo", detail: "Walkthrough · 1:12", src: "xone-product-demo.mp4", poster: "xone-product-demo-poster.jpg" },
];

const chapters = [
  {
    number: "01",
    title: "Write your Signal brief.",
    body: "Describe what belongs in your Xone and what to leave out. Write it yourself or sharpen your draft with AI.",
  },
  {
    number: "02",
    title: "Choose the sources.",
    body: "Pick up to ten sources per Xone, get AI recommendations, or add your own RSS feeds. Xone refreshes the sources you assign.",
  },
  {
    number: "03",
    title: "Find your focus.",
    body: "Choose Broad, Focused, or Laser-focused. Open a headline to read at its source, or start with a cited TL:DR briefing.",
  },
];

// # github repo here — configured for the project repository.
const githubRepoUrl = "https://github.com/pkscanvas/xone";
// # download link here — available after publishing the first GitHub Release.
const downloadUrl = "https://github.com/pkscanvas/xone/releases/latest/download/Xone_0.1.0_aarch64.dmg";

function resolveProjectUrl() {
  if (githubRepoUrl) return githubRepoUrl;
  if (import.meta.env.VITE_GITHUB_URL) return import.meta.env.VITE_GITHUB_URL;
  if (typeof window === "undefined") return null;

  const { hostname, pathname } = window.location;
  if (hostname.endsWith("github.io")) {
    const owner = hostname.split(".")[0];
    const repository = pathname.split("/").filter(Boolean)[0];
    if (owner && repository) return `https://github.com/${owner}/${repository}`;
  }

  return null;
}

function Brand() {
  return (
    <a className="brand" href="#top" aria-label="Xone home">
      <img src="./assets/xone-icon.png" alt="" />
      <span>Xone</span>
    </a>
  );
}

function ActionLink({ children, className = "", href, icon: Icon, download = false }) {
  const external = href.startsWith("http");
  return (
    <a
      className={`button ${className}`.trim()}
      href={href}
      download={download || undefined}
      {...(external && !download ? { target: "_blank", rel: "noreferrer" } : {})}
    >
      <span>{children}</span>
      {Icon ? <Icon aria-hidden="true" size={17} weight="bold" /> : null}
    </a>
  );
}

export function App() {
  const [menuOpen, setMenuOpen] = useState(false);
  const [preview, setPreview] = useState(0);
  const [activeVideo, setActiveVideo] = useState(0);
  const video = videos[activeVideo];
  const projectUrl = useMemo(resolveProjectUrl, []);
  const githubUrl = projectUrl || "#github-repo-here";
  const releasesUrl = projectUrl ? `${projectUrl.replace(/\/$/, "")}/releases/latest` : "#github-repo-here";
  const installerUrl = downloadUrl || import.meta.env.VITE_DOWNLOAD_URL || (projectUrl ? `${releasesUrl}/download/Xone_0.1.0_aarch64.dmg` : "#download-link-here");

  const closeMenu = () => setMenuOpen(false);

  return (
    <div className="site-shell" id="top">
      <a className="skip-link" href="#main">
        Skip to content
      </a>

      <header className="site-header">
        <Brand />
        <nav className="desktop-nav" aria-label="Primary navigation">
          <a href="#product">Product</a>
          <a href="#features">Features</a>
          <a href="#sources">Sources</a>
          <a href="#intelligence">Intelligence</a>
          <a href="#roadmap">Roadmap</a>
        </nav>
        <ActionLink className="header-cta" href={installerUrl} icon={DownloadSimple} download>
          Get Xone
        </ActionLink>
        <button
          className="menu-button"
          type="button"
          aria-expanded={menuOpen}
          aria-controls="mobile-menu"
          aria-label={menuOpen ? "Close menu" : "Open menu"}
          onClick={() => setMenuOpen((open) => !open)}
        >
          {menuOpen ? <X size={24} /> : <List size={24} />}
        </button>
        <nav
          className={`mobile-nav ${menuOpen ? "is-open" : ""}`}
          id="mobile-menu"
          aria-label="Mobile navigation"
        >
          <a href="#product" onClick={closeMenu}>Product</a>
          <a href="#features" onClick={closeMenu}>Features</a>
          <a href="#sources" onClick={closeMenu}>Sources</a>
          <a href="#intelligence" onClick={closeMenu}>Intelligence</a>
          <a href="#roadmap" onClick={closeMenu}>Roadmap</a>
          <a href={githubUrl} onClick={closeMenu}>GitHub</a>
          <a href={installerUrl} download onClick={closeMenu}>Get Xone</a>
        </nav>
      </header>

      <main id="main">
        <section className="hero section-frame" id="product">
          <div className="hero-copy">
            <p className="eyebrow">AI-FILTERED RSS · DESKTOP NEWS READER</p>
            <h1>
              Less Feed.<br />
              <em>More Signal.</em>
            </h1>
            <span className="burgundy-rule" aria-hidden="true" />
            <p className="hero-description">
              A news reader that follows your brief. Turn RSS feeds into focused
              Xones, bring related stories together, and catch up with cited AI briefings.
            </p>
            <div className="hero-actions">
              <ActionLink className="button-dark" href={installerUrl} icon={DownloadSimple} download>
                Download for macOS
              </ActionLink>
              <ActionLink className="button-light" href={githubUrl} icon={GithubLogo}>
                View on GitHub
              </ActionLink>
            </div>
            <p className="platform-note">Free & open source · macOS on Apple Silicon</p>
            <a className="setup-link" href="#setup">Bring a local or cloud AI provider <ArrowDown size={13} aria-hidden="true" /></a>
          </div>

          <figure className="hero-product">
            <div className="media-heading"><span>WATCH XONE</span><span>{activeVideo + 1} / {videos.length} VIDEOS</span></div>
            <video
              key={video.src}
              id="hero-video"
              controls
              playsInline
              preload="metadata"
              poster={`./assets/${video.poster}`}
              aria-label={video.title}
            >
              <source src={`./assets/${video.src}`} type="video/mp4" />
              Your browser does not support embedded video.
            </video>
            <div className="video-choices" role="group" aria-label="Choose from two Xone videos">
              {videos.map((item, index) => (
                <button
                  key={item.src}
                  type="button"
                  aria-pressed={activeVideo === index}
                  aria-controls="hero-video"
                  onClick={() => setActiveVideo(index)}
                >
                  <img src={`./assets/${item.poster}`} alt="" width="1920" height="1080" />
                  <span className="video-choice-copy"><strong>{String(index + 1).padStart(2, "0")} · {item.title}</strong><span>{item.detail}</span></span>
                </button>
              ))}
            </div>
            <figcaption className="video-credit">
              Music: 'Open Spaces' by Hybridas on #Uppbeat <a href="https://uppbeat.io/c/hybridas" target="_blank" rel="noreferrer">https://uppbeat.io/c/hybridas</a>
            </figcaption>
          </figure>
        </section>

        <div className="ticker" aria-label="Xone highlights">
          <span>494 CURATED SOURCES</span>
          <span>7 AI PROVIDERS</span>
          <span>3 STARTER XONES</span>
          <span>CITED TL:DR BRIEFINGS</span>
        </div>

        <section className="features section-frame" id="features" aria-labelledby="features-heading">
          <div className="feature-intro">
            <div><p className="section-label">/ IN THE APP TODAY</p><h2 id="features-heading">Less sorting.<br /><em>More understanding.</em></h2></div>
            <p>From the first filter to the daily catch-up, every feature helps you stay close to the stories you care about.</p>
          </div>
          <div className="feature-grid">
            {features.map(({ icon: Icon, title, body }) => (
              <article className="feature" key={title}>
                <Icon size={26} weight="regular" aria-hidden="true" />
                <h3>{title}</h3><p>{body}</p>
              </article>
            ))}
          </div>
        </section>

        <section className="signal section-frame" id="sources">
          <div className="section-intro">
            <p className="section-label">/ YOUR READING ROOM</p>
            <h2>
              From everything<br />to only what matters.
            </h2>
            <p>
              Start with AI, Open Source AI, and Design, or create your own Xones.
              Choose from 494 sources and let your Signal brief shape each stream.
            </p>
            <p>Browse sources by region or topic. Read in the style that suits you, with your Xones always a click away.</p>
            <span className="burgundy-rule" aria-hidden="true" />
          </div>
          <figure className="signal-visual">
            <div className="preview-controls" role="group" aria-label="App screenshot previews">
              {previews.map(({ label }, index) => <button key={label} type="button" aria-pressed={preview === index} onClick={() => setPreview(index)}>{label}</button>)}
              <span>EXPLORE XONE</span>
            </div>
            <img
              src={`./assets/${previews[preview].image}`}
              alt={previews[preview].alt}
              width="1600" height="1000" loading="lazy"
            />
            <figcaption>App preview · illustrative article content</figcaption>
          </figure>
        </section>

        <section className="chapters section-frame" aria-labelledby="workflow-heading">
          <p className="section-label">/ HOW IT WORKS</p>
          <h2 className="visually-hidden" id="workflow-heading">How Xone works</h2>
          {chapters.map((chapter) => (
            <article className="chapter" key={chapter.number}>
              <span className="chapter-number">{chapter.number}</span>
              <h3>{chapter.title}</h3>
              <p>{chapter.body}</p>
            </article>
          ))}
        </section>

        <section className="providers" id="intelligence">
          <div className="providers-heading">
            <p className="section-label">/ PROVIDER CHOICE</p>
            <h2>Your provider.<br />Your rules.</h2>
            <p>Connect Ollama or LM Studio for local AI, or choose a cloud provider. Each provider keeps its own model, endpoint, and key settings.</p>
          </div>
          <div className="provider-list" aria-label="Supported AI providers">
            {providers.map(({ name, detail, icon: Icon }) => (
              <div className="provider" key={name}>
                <Icon size={36} weight="regular" aria-hidden="true" />
                <strong>{name}</strong>
                <span>{detail}</span>
              </div>
            ))}
          </div>
          <p className="provider-statement">Your choice of provider. Your choice of model.</p>
        </section>

        <section className="proof section-frame">
          <div className="proof-copy">
            <p className="section-label burgundy">/ KNOW WHAT CONNECTS</p>
            <h2>Local or cloud.<br />You decide.</h2>
            <p>
              Your sources, reading history, and settings live on your device.
              When you use cloud AI, the selected provider receives the headlines,
              excerpts, and instructions needed for the feature you use.
            </p>
            <a href="#intelligence" className="text-link">
              Learn about providers <ArrowDown size={15} aria-hidden="true" />
            </a>
          </div>
          <figure className="proof-image">
            <img
              src="./assets/provider-settings.png"
              alt="Xone AI provider settings with separate model and API key controls"
              loading="lazy"
            />
          </figure>
        </section>

        <section className="roadmap section-frame" id="roadmap" aria-labelledby="roadmap-heading">
          <div className="roadmap-intro">
            <p className="section-label">/ WHAT’S NEXT</p>
            <h2 id="roadmap-heading">A smaller model.<br /><em>A bigger step forward.</em></h2>
            <p>Moving toward intelligence built for the task, and built into Xone.</p>
            <span className="roadmap-note">Planned · not available in the current release</span>
          </div>
          <ol className="roadmap-list">
            <li>
              <span className="roadmap-number">01</span>
              <div><span className="status-label">PLANNED</span><h3>Support for decision models.</h3>
                <p>Add support for models such as Jev to make focused relevance decisions about which articles belong in each Xone.</p>
              </div>
            </li>
            <li>
              <span className="roadmap-number">02</span>
              <div><span className="status-label">PLANNED</span><h3>A model made for Xone.</h3>
                <p>Fine-tune a decision model for article relevance and bundle it with the app. The goal: faster, more accurate filtering with no external model or separate model setup required for that task.</p>
              </div>
            </li>
          </ol>
        </section>

        <section className="setup section-frame" id="setup" aria-labelledby="setup-heading">
          <div><p className="section-label">/ GET STARTED</p><h2 id="setup-heading">Your first Xone<br />is already waiting.</h2></div>
          <ol>
            <li><strong>Install Xone.</strong> Download the Apple Silicon DMG and drag Xone into Applications. If macOS blocks the first launch, follow the included Read Me First guide to Privacy & Security → Open Anyway.</li>
            <li><strong>Connect your AI.</strong> Choose a local server or save your cloud provider’s key in Settings. The current app requires an external AI model; none is bundled.</li>
            <li><strong>Make it yours.</strong> Start with AI, Open Source AI, and Design, or create a Xone around your own Signal brief.</li>
          </ol>
        </section>

        <section className="closing-cta section-frame">
          <h2>Make room for the<br />news that matters.</h2>
          <div className="closing-actions">
            <ActionLink className="button-dark" href={installerUrl} icon={DownloadSimple} download>
              Download for macOS
            </ActionLink>
            <ActionLink className="button-light" href={githubUrl} icon={ArrowUpRight}>
              View on GitHub
            </ActionLink>
          </div>
        </section>
      </main>

      <footer className="site-footer section-frame">
        <div className="footer-brand">
          <Brand />
        </div>
        <div className="footer-column">
          <strong>PRODUCT</strong>
          <a href="#product">Overview</a>
          <a href="#sources">Sources</a>
          <a href="#intelligence">Providers</a>
          <a href="#roadmap">Roadmap</a>
        </div>
        <div className="footer-column">
          <strong>PROJECT</strong>
          <a href={githubUrl}>GitHub</a>
          <a href={projectUrl ? `${projectUrl.replace(/\/$/, "")}/issues` : "#github-repo-here"}>Issues</a>
          <a href={releasesUrl}>Releases</a>
        </div>
        <p className="project-note">
          <a href={githubUrl}>Open source on GitHub</a>
          <br />Xone · 2026
        </p>
      </footer>
    </div>
  );
}
