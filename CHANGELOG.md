# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
