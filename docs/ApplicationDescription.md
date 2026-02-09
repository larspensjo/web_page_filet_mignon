# Application description

The goal of the application is to autonomously collect and prioritize web content about AI companies, then produce a concise, high-quality morning briefing.

## Core goals
1. Load URLs from RSS feeds and other automated sources.
2. Fetch and extract clean text from the discovered pages.
3. Use AI with a configured prompt to filter and rank which pages are interesting.
4. Produce a top list (for example the top 10 pages) and an AI-generated executive summary.
5. Run unattended so results are ready in the morning.

## Typical workflow
1. The app starts on a schedule or at system startup.
2. It collects new links from RSS feeds.
3. It downloads pages, extracts text, and normalizes content.
4. The AI filter selects and ranks the most relevant pages.
5. The app generates an executive summary across the selected pages.
6. The output is available for review when the user starts the day.

## Output
- A ranked list of selected pages (top N).
- An executive summary derived from the selected pages.
- Optional previews for spot-checking extraction and ranking quality.

## Design principles
- Fully automated by default, with manual review as a secondary option.
- Deterministic processing and clear provenance for reproducible results.
- Strong security boundaries for untrusted web content and AI output.
