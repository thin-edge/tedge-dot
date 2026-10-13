## ADDED Requirements

### Requirement: Highlights are fragments in changes/
A user-visible change that needs more than its changelog line SHALL add one Markdown file to
`changes/`, starting with a `### ` heading. `packaging/release-notes.md` SHALL be the fixed
header of every release and SHALL NOT carry feature descriptions.

#### Scenario: Feature with a highlight
- **WHEN** a pull request adds a feature and `changes/opcua-method-calls.md`
- **THEN** the next release's notes include that highlight, and `packaging/release-notes.md` is unchanged

### Requirement: A release shows only its own highlights
The release notes SHALL contain the fixed header, then the highlights added between the
previous published release and the release commit, in the order they were added, then the
changelog sections from the release back to the previous published release. A release is
published when its GitHub release has assets; a tag whose release run failed is not, and the
next published release SHALL carry its highlights and changelog and say that it does.
A highlight edited or kept after a release SHALL NOT appear on a later release. A highlight added
and removed before the release SHALL NOT appear. Re-running the release workflow SHALL NOT
duplicate the header.

#### Scenario: Bug-fix release
- **WHEN** a release contains only a fix and no new file in `changes/`
- **THEN** its notes contain the header and the changelog, and no Highlights section

#### Scenario: Previous tag never published
- **WHEN** 0.0.15 was tagged but its release has no assets, and 0.0.16 is released
- **THEN** the 0.0.16 notes include the highlights added since 0.0.14, the 0.0.16 and 0.0.15 changelog sections, and a note that 0.0.15 was not published

#### Scenario: Re-run
- **WHEN** the release workflow is run again for an existing tag
- **THEN** the notes are regenerated with one header, the same highlights and the same changelog

### Requirement: What's new page
The site SHALL have a What's new page that lists every highlight under the release that first
contained it, newest release first, with a link to the GitHub release. Highlights not yet in a
release SHALL appear under "Unreleased".

#### Scenario: Highlight released
- **WHEN** `changes/foo.md` was added before tag `0.0.17` and after `0.0.16`
- **THEN** the What's new page shows it under 0.0.17
