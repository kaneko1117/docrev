# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.3.0](https://github.com/kaneko1117/docrev/compare/v0.2.0...v0.3.0) - 2026-08-22

### Added

- read the workbook's own Excel comments ([#53](https://github.com/kaneko1117/docrev/pull/53))
- show formulas in the formula bar and dump --formulas ([#50](https://github.com/kaneko1117/docrev/pull/50))
- mouse support — click, wheel, drag-to-copy TSV ([#49](https://github.com/kaneko1117/docrev/pull/49))
- open the comment panel on demand, not on every cursor move ([#48](https://github.com/kaneko1117/docrev/pull/48))
- carry the anchored cell's content in comment list --json ([#55](https://github.com/kaneko1117/docrev/pull/55))
- honor frozen panes from the workbook ([#52](https://github.com/kaneko1117/docrev/pull/52))
- find on the active sheet with Ctrl+F ([#54](https://github.com/kaneko1117/docrev/pull/54))
- jump to a sheet by name with Ctrl+G ([#51](https://github.com/kaneko1117/docrev/pull/51))

### Changed

- split app/viewer.rs into a directory module per mode ([#61](https://github.com/kaneko1117/docrev/pull/61))
- split ui/grid.rs into per-widget modules ([#60](https://github.com/kaneko1117/docrev/pull/60))

### Documentation

- drop version and progress snapshots, record release lessons ([#68](https://github.com/kaneko1117/docrev/pull/68))

### Fixed

- phonetic runs and out-of-range anchors in workbook comments

## [0.2.0](https://github.com/kaneko1117/docrev/compare/v0.1.0...v0.2.0) - 2026-08-15

### Added

- show where you are in a wide workbook
- add a --theme flag for terminal-native colors
- reload comments automatically instead of on F5
- dock the comment editor into a proportional sidebar
- wrap long cell text within its column
- inherit font colors from the workbook
- inherit cell fill colors from the workbook
- wire number formats from styles.xml into the viewer
- Excel number-format engine (practical subset)
- render merged regions as single cells
- inherit workbook column widths and overflow cell text

### Changed

- split workbook semantics, layout and rendering by layer

### Fixed

- a reply reopens a resolved thread
- make the terminal theme readable on real color schemes
- stop a save from hiding an agent's concurrent write
- harden format wiring against hostile and nonstandard files
- address adversarial-review findings
- trim zip to the deflate feature only
- make CLI test temp files unique per test
