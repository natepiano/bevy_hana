# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Initial release. A Bevy kernel for durable device identity, presence,
  availability, and recovery policy. Hardware providers perform I/O and report
  their full device set to this crate; the crate does not enumerate or operate
  hardware itself.
