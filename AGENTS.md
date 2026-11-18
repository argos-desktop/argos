# Working on Argos

Start by reading [HANDOFF.md](HANDOFF.md) and checking the current branch, working tree, and recent commits. Verify the handoff against the checkout. Preserve work in progress, including uncommitted changes made in another session.

Argos is an independently implemented desktop assistant for Windows and macOS. Develop and test on the available Mac while Windows hardware validation is pending. Linux is a later target. Write original code. Claude and Codex are the primary assistants users will choose between inside Argos. Prefer user-supplied Anthropic and OpenAI API keys respectively. Follow the design’s provider-specific subscription options and require explicit selection before changing the billing source. Keep real keys, captured media, transcripts, and private chat logs out of source control, test fixtures, and diagnostics.

Keep decisions and commands that both coding tools need in the repository. When stopping after meaningful work, refresh HANDOFF.md with the current milestone, unfinished work, checks actually run and their outcomes, and the next actionable step. Keep it short. Put lasting design decisions in the design document it links to.
