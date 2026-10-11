// Public starter definitions copied from the installed app on 2026-10-04.
// Deliberately excludes settings, credentials, articles, and learned behavior.
pub struct DefaultZone {
    pub name: &'static str,
    pub filter: &'static str,
    pub focus: &'static str,
    pub feed_urls: &'static [&'static str],
}

pub const DEFAULT_ZONES: &[DefaultZone] = &[
    DefaultZone {
        name: "AI",
        filter: "Include articles whose primary subject is the release, launch, update, new or changed capabilities, or availability of an AI model, large language model (LLM), or generative AI model. Exclude articles where AI models, LLMs, or generative AI are mentioned but are not the primary subject. Exclude articles primarily about open-source or open-weight AI models, LLMs, or generative AI models.",
        focus: "focused",
        feed_urls: &[
            "https://feeds.arstechnica.com/arstechnica/technology-lab",
            "https://hnrss.org/frontpage",
            "https://huggingnews.com/feed.xml",
            "https://techcrunch.com/feed/",
            "https://thenewstack.io/feed/",
            "https://www.technologyreview.com/topic/artificial-intelligence/feed/",
            "https://www.theverge.com/rss/ai-artificial-intelligence/index.xml",
            "https://www.wired.com/feed/rss",
        ],
    },
    DefaultZone {
        name: "Open Source AI",
        filter: "Include articles primarily about open-source large language models, including their development, releases, capabilities, research, adoption, governance, licensing, or ecosystem. Exclude articles primarily about proprietary large language models, proprietary AI tools or products, or generic company announcements, tool launches, or coverage not centered on an open-source large language model.",
        focus: "focused",
        feed_urls: &[
            "https://huggingface.co/blog/feed.xml",
            "https://huggingnews.com/feed.xml",
            "https://lastweekin.ai/feed",
            "https://thegradient.pub/rss/",
            "https://thenewstack.io/feed/",
            "https://www.livemint.com/rss/AI",
            "https://www.technologyreview.com/topic/artificial-intelligence/feed/",
            "https://www.theverge.com/rss/ai-artificial-intelligence/index.xml",
        ],
    },
    DefaultZone {
        name: "Design",
        filter: "Include content exclusively about graphic design, including visual communication design, typography, layout design, branding, and related software/tools. Exclude articles about unrelated design fields like interior design, fashion design, or industrial design.",
        focus: "focused",
        feed_urls: &[
            "https://feeds.feedburner.com/design-milk",
            "https://gdusa.com/feed/",
            "https://inspireme.blog/category/motion-graphics/feed/",
            "https://motionographer.com/feed/",
            "https://www.creativebloq.com/feeds/tag/graphic-design",
            "https://www.designer-daily.com/tag/graphic-design/feed",
            "https://www.dexigner.com/feed/news/Graphic-Design",
            "https://www.premiumbeat.com/blog/category/motion-graphics-2/feed/",
        ],
    },
];
