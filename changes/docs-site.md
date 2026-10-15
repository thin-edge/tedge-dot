### A documentation site

tedge-dot now has a documentation site at <https://thin-edge.github.io/tedge-dot/>: task-first
guides for every protocol, the reference pages rendered from `doc/`, and three tools that run in
the browser:

- a **config builder** that writes a connector config and checks it against the contract schema
  as you type;
- a **package picker** that says whether `tedge-dot-rs` or `tedge-dot-c` suits your gateway, and
  why;
- a **report-policy playground** that shows what a `report` table publishes on a signal of your
  choice, using the same rules (and test vectors) as the connectors.

The Markdown in the repository stays the source: edit `doc/` and the site follows.
