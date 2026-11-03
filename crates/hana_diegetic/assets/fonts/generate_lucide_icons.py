#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.12"
# dependencies = [
#   "blake3>=1.0.0,<2",
# ]
# ///
"""Download a pinned lucide-static release and generate Hana's icon enum."""

from __future__ import annotations

import argparse
import base64
import hashlib
import hmac
import importlib
import io
import json
import re
import sys
import tarfile
import urllib.error
import urllib.request
from pathlib import Path
from typing import Final, Protocol, TypedDict, cast


EXPECTED_CANONICAL_ICON_COUNT: Final = 1_870
EXPECTED_FONT_NAME_COUNT: Final = 2_134
EXPECTED_CODEPOINT_NAME_COUNT: Final = 2_154
EXPECTED_ALIAS_COUNT: Final = 264
EXPECTED_NAMES_MISSING_FROM_FONT: Final = frozenset(
    {
        "chrome",
        "chromium",
        "circle-euro-sign",
        "codepen",
        "codesandbox",
        "dribbble",
        "facebook",
        "figma",
        "framer",
        "github",
        "gitlab",
        "instagram",
        "linkedin",
        "pocket",
        "rail-symbol",
        "slack",
        "trello",
        "twitch",
        "twitter",
        "youtube",
    }
)
PACKAGE_ROOT: Final = "package"
REGISTRY_ROOT: Final = "https://registry.npmjs.org/lucide-static"
SUPPORTED_NAME: Final = re.compile(r"[a-z0-9]+(?:-[a-z0-9]+)*")
UNICODE_ENTITY: Final = re.compile(r"&#([0-9]+);")


class FontInfoEntry(TypedDict):
    """The fields supplied for one glyph in `font/info.json`."""

    encodedCode: str
    prefix: str
    className: str
    unicode: str


class RegistryDistribution(TypedDict):
    """The authenticated archive fields in an npm version response."""

    integrity: str
    tarball: str


class RegistryVersionMetadata(TypedDict):
    """The npm metadata needed to authenticate one lucide-static archive."""

    version: str
    dist: RegistryDistribution


class IconDefinition(TypedDict):
    """One validated canonical icon ready for Rust generation."""

    package_name: str
    api_name: str
    variant: str
    codepoint: int


class Arguments(argparse.Namespace):
    """Typed command-line arguments for one regeneration."""

    def __init__(self) -> None:
        super().__init__()
        self.version: str = ""


class GenerationError(RuntimeError):
    """An upstream mismatch that must stop generation."""


class DownloadResponse(Protocol):
    """The response operations needed to consume one package download."""

    def read(self) -> bytes:
        """Read the complete response body."""
        ...

    def close(self) -> None:
        """Release the response's network resources."""
        ...


class ContentDigest(Protocol):
    """A computed content digest that can produce hexadecimal text."""

    def hexdigest(self) -> str:
        """Return the digest as lowercase hexadecimal text."""
        ...


class Blake3Package(Protocol):
    """The one operation used from the generator's declared dependency."""

    def blake3(self, data: bytes) -> ContentDigest:
        """Hash a byte string with BLAKE3."""
        ...


def download(url: str) -> bytes:
    """Download one URL and return its complete response body."""
    request = urllib.request.Request(
        url,
        headers={"User-Agent": "hana-diegetic-lucide-generator/1"},
    )
    response = cast(
        DownloadResponse, urllib.request.urlopen(request, timeout=60)
    )
    try:
        return response.read()
    finally:
        response.close()


def parse_json_object(payload: bytes, source_name: str) -> dict[str, object]:
    """Parse a JSON object while preserving an object-only type boundary."""
    decoded = cast(object, json.loads(payload))
    if not isinstance(decoded, dict):
        raise GenerationError(f"{source_name} must contain a JSON object")
    object_values = cast(dict[object, object], decoded)
    if not all(isinstance(key, str) for key in object_values):
        raise GenerationError(f"{source_name} contains a non-string key")
    return cast(dict[str, object], object_values)


def required_string(
    values: dict[str, object], key: str, source_name: str
) -> str:
    """Read one required string from a validated JSON object."""
    value = values.get(key)
    if not isinstance(value, str):
        raise GenerationError(f"{source_name}.{key} must be a string")
    return value


def parse_registry_metadata(
    payload: bytes, requested_version: str
) -> RegistryVersionMetadata:
    """Validate the npm fields used to authenticate the release archive."""
    values = parse_json_object(payload, "npm metadata")
    version = required_string(values, "version", "npm metadata")
    if version != requested_version:
        raise GenerationError(
            f"npm returned version {version!r} for requested version {requested_version!r}"
        )

    distribution_value = values.get("dist")
    if not isinstance(distribution_value, dict):
        raise GenerationError("npm metadata.dist must be an object")
    distribution_values = cast(dict[str, object], distribution_value)
    distribution: RegistryDistribution = {
        "integrity": required_string(
            distribution_values, "integrity", "npm metadata.dist"
        ),
        "tarball": required_string(
            distribution_values, "tarball", "npm metadata.dist"
        ),
    }
    return {"version": version, "dist": distribution}


def verify_archive_integrity(payload: bytes, integrity: str) -> None:
    """Verify an archive against npm's SHA-512 subresource integrity value."""
    algorithm, separator, expected_digest = integrity.partition("-")
    if separator != "-" or algorithm != "sha512" or not expected_digest:
        raise GenerationError(
            f"unsupported npm integrity value {integrity!r}; expected sha512"
        )
    actual_digest = base64.b64encode(hashlib.sha512(payload).digest()).decode("ascii")
    if not hmac.compare_digest(actual_digest, expected_digest):
        raise GenerationError("downloaded archive does not match npm integrity")


def read_archive_file(archive: tarfile.TarFile, relative_path: str) -> bytes:
    """Read one regular file from the package without extracting the archive."""
    archive_path = f"{PACKAGE_ROOT}/{relative_path}"
    try:
        member = archive.getmember(archive_path)
    except KeyError as error:
        raise GenerationError(f"archive is missing {archive_path}") from error
    if not member.isfile():
        raise GenerationError(f"archive member {archive_path} is not a regular file")
    extracted = archive.extractfile(member)
    if extracted is None:
        raise GenerationError(f"could not read archive member {archive_path}")
    with extracted:
        return extracted.read()


def parse_font_info(payload: bytes) -> dict[str, FontInfoEntry]:
    """Validate and type every glyph entry from `font/info.json`."""
    values = parse_json_object(payload, "font/info.json")
    entries: dict[str, FontInfoEntry] = {}
    for name, entry_value in values.items():
        if not isinstance(entry_value, dict):
            raise GenerationError(f"font/info.json entry {name!r} must be an object")
        entry_values = cast(dict[str, object], entry_value)
        entries[name] = {
            "encodedCode": required_string(
                entry_values, "encodedCode", f"font/info.json[{name!r}]"
            ),
            "prefix": required_string(
                entry_values, "prefix", f"font/info.json[{name!r}]"
            ),
            "className": required_string(
                entry_values, "className", f"font/info.json[{name!r}]"
            ),
            "unicode": required_string(
                entry_values, "unicode", f"font/info.json[{name!r}]"
            ),
        }
    return entries


def parse_canonical_names(payload: bytes) -> set[str]:
    """Read the canonical icon names from `icon-nodes.json`."""
    return set(parse_json_object(payload, "icon-nodes.json"))


def parse_codepoints(payload: bytes) -> dict[str, int]:
    """Validate the package-wide name-to-codepoint map."""
    values = parse_json_object(payload, "font/codepoints.json")
    codepoints: dict[str, int] = {}
    for name, value in values.items():
        if not isinstance(value, int) or isinstance(value, bool):
            raise GenerationError(
                f"font/codepoints.json entry {name!r} must be an integer"
            )
        codepoints[name] = value
    return codepoints


def parse_unicode_value(name: str, entry: FontInfoEntry) -> int:
    """Convert one validated info entry's HTML numeric entity to a codepoint."""
    matched = UNICODE_ENTITY.fullmatch(entry["unicode"])
    if matched is None:
        raise GenerationError(
            f"font/info.json entry {name!r} has invalid Unicode value {entry['unicode']!r}"
        )
    value = int(matched.group(1))
    try:
        _ = chr(value)
    except ValueError as error:
        raise GenerationError(
            f"font/info.json entry {name!r} has invalid codepoint {value}"
        ) from error
    return value


def rust_variant(name: str) -> str:
    """Convert one canonical lowercase kebab name to its Rust enum variant."""
    if SUPPORTED_NAME.fullmatch(name) is None:
        raise GenerationError(f"canonical icon has unsupported name {name!r}")
    if name[0].isdigit():
        raise GenerationError(f"canonical icon name starts with a digit: {name!r}")
    return "".join(part[0].upper() + part[1:] for part in name.split("-"))


def validate_sources(
    canonical_names: set[str],
    font_info: dict[str, FontInfoEntry],
    package_codepoints: dict[str, int],
) -> list[IconDefinition]:
    """Check Lucide's expected source relationships and build icon definitions."""
    if len(canonical_names) != EXPECTED_CANONICAL_ICON_COUNT:
        raise GenerationError(
            f"icon-nodes.json has {len(canonical_names)} canonical icons; expected "
            + str(EXPECTED_CANONICAL_ICON_COUNT)
        )
    if len(font_info) != EXPECTED_FONT_NAME_COUNT:
        raise GenerationError(
            f"font/info.json has {len(font_info)} names; expected {EXPECTED_FONT_NAME_COUNT}"
        )
    if len(package_codepoints) != EXPECTED_CODEPOINT_NAME_COUNT:
        raise GenerationError(
            f"font/codepoints.json has {len(package_codepoints)} names; expected "
            + str(EXPECTED_CODEPOINT_NAME_COUNT)
        )

    font_names = set(font_info)
    package_names = set(package_codepoints)
    missing_canonical_names = canonical_names - font_names
    if missing_canonical_names:
        raise GenerationError(
            "canonical icons missing from font/info.json: "
            + ", ".join(sorted(missing_canonical_names))
        )

    aliases = font_names - canonical_names
    if len(aliases) != EXPECTED_ALIAS_COUNT:
        raise GenerationError(
            f"font contains {len(aliases)} non-canonical aliases; expected {EXPECTED_ALIAS_COUNT}"
        )

    names_missing_from_font = package_names - font_names
    if names_missing_from_font != set(EXPECTED_NAMES_MISSING_FROM_FONT):
        raise GenerationError(
            "font/codepoints.json names absent from the font changed: "
            + ", ".join(sorted(names_missing_from_font))
        )
    unexpected_font_names = font_names - package_names
    if unexpected_font_names:
        raise GenerationError(
            "font/info.json names absent from font/codepoints.json: "
            + ", ".join(sorted(unexpected_font_names))
        )

    definitions: list[IconDefinition] = []
    variant_sources: dict[str, str] = {}
    api_name_sources: dict[str, str] = {}
    for package_name in sorted(canonical_names):
        api_name = package_name
        previous_package_name = api_name_sources.get(api_name)
        if previous_package_name is not None:
            raise GenerationError(
                f"package icon names {previous_package_name!r} and {package_name!r} "
                + f"both use API name {api_name!r}"
            )
        api_name_sources[api_name] = package_name

        variant = rust_variant(api_name)
        previous_name = variant_sources.get(variant)
        if previous_name is not None:
            raise GenerationError(
                f"canonical icon names {previous_name!r} and {package_name!r} "
                + f"both convert to {variant!r}"
            )
        variant_sources[variant] = package_name

        codepoint = parse_unicode_value(package_name, font_info[package_name])
        package_codepoint = package_codepoints.get(package_name)
        if package_codepoint != codepoint:
            raise GenerationError(
                f"codepoint disagreement for {package_name!r}: info.json has {codepoint}, "
                + f"codepoints.json has {package_codepoint}"
            )
        definitions.append(
            {
                "package_name": package_name,
                "api_name": api_name,
                "variant": variant,
                "codepoint": codepoint,
            }
        )
    return definitions


def render_generated_rust(
    version: str, font_digest: str, icons: list[IconDefinition]
) -> str:
    """Render the deterministic Rust API for one validated Lucide release."""
    lines = [
        "// @generated by crates/hana_diegetic/assets/fonts/generate_lucide_icons.py",
        f"// from lucide-static {version}. Do not edit by hand.",
        "// Regenerate: uv run crates/hana_diegetic/assets/fonts/"
        + f"generate_lucide_icons.py --version {version}",
        "",
        "/// The `lucide-static` version this file and `assets/fonts/lucide.ttf` come from.",
        "#[cfg(test)]",
        f'pub(super) const LUCIDE_VERSION: &str = "{version}";',
        "",
        "/// BLAKE3 of that version's `font/lucide.ttf`.",
        "#[cfg(test)]",
        "pub(super) const LUCIDE_FONT_BLAKE3: &str =",
        f'    "{font_digest}";',
        "",
        "/// A canonical icon from the bundled Lucide font.",
        "#[derive(Clone, Copy, Debug, Eq, PartialEq)]",
        "pub enum LucideIcon {",
    ]
    for icon in icons:
        lines.extend(
            [
                f"    /// The `{icon['api_name']}` Lucide icon.",
                f"    {icon['variant']},",
            ]
        )
    lines.extend(
        [
            "}",
            "",
            "#[cfg(test)]",
            f"pub(super) const ALL: [LucideIcon; {len(icons)}] = [",
        ]
    )
    lines.extend(f"    LucideIcon::{icon['variant']}," for icon in icons)
    lines.extend(
        [
            "];",
            "",
            "impl LucideIcon {",
            "    #[must_use]",
            "    #[expect(",
            "        clippy::too_many_lines,",
            '        reason = "the generated match names every canonical Lucide icon"',
            "    )]",
            "    pub(crate) const fn codepoint(self) -> char {",
            "        match self {",
        ]
    )
    lines.extend(
        f"            Self::{icon['variant']} => '\\u{{{icon['codepoint']:x}}}',"
        for icon in icons
    )
    lines.extend(
        [
            "        }",
            "    }",
            "",
            "    /// Returns this icon's canonical kebab-case Lucide name.",
            "    #[must_use]",
            "    #[expect(",
            "        clippy::too_many_lines,",
            '        reason = "the generated match names every canonical Lucide icon"',
            "    )]",
            "    pub const fn name(self) -> &'static str {",
            "        match self {",
        ]
    )
    lines.extend(
        f'            Self::{icon["variant"]} => "{icon["api_name"]}",'
        for icon in icons
    )
    lines.extend(["        }", "    }", "}", ""])
    return "\n".join(lines)


def generate(version: str) -> None:
    """Download, validate, and write all artifacts for one Lucide version."""
    if re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", version) is None:
        raise GenerationError(f"version must be a numeric semver, got {version!r}")

    archive_url = f"{REGISTRY_ROOT}/-/lucide-static-{version}.tgz"
    metadata = parse_registry_metadata(
        download(f"{REGISTRY_ROOT}/{version}"), version
    )
    if metadata["dist"]["tarball"] != archive_url:
        raise GenerationError(
            f"npm metadata names an unexpected tarball: {metadata['dist']['tarball']!r}"
        )

    archive_payload = download(archive_url)
    verify_archive_integrity(archive_payload, metadata["dist"]["integrity"])
    with tarfile.open(fileobj=io.BytesIO(archive_payload), mode="r:gz") as archive:
        font_payload = read_archive_file(archive, "font/lucide.ttf")
        license_payload = read_archive_file(archive, "LICENSE")
        font_info = parse_font_info(read_archive_file(archive, "font/info.json"))
        canonical_names = parse_canonical_names(
            read_archive_file(archive, "icon-nodes.json")
        )
        package_codepoints = parse_codepoints(
            read_archive_file(archive, "font/codepoints.json")
        )

    icons = validate_sources(canonical_names, font_info, package_codepoints)
    blake3_package = cast(
        Blake3Package, cast(object, importlib.import_module("blake3"))
    )
    font_digest = blake3_package.blake3(font_payload).hexdigest()
    generated_rust = render_generated_rust(version, font_digest, icons)

    script_path = Path(__file__).resolve()
    font_directory = script_path.parent
    crate_root = font_directory.parents[1]
    icon_directory = crate_root / "src" / "icon"
    icon_directory.mkdir(parents=True, exist_ok=True)
    _ = (font_directory / "lucide.ttf").write_bytes(font_payload)
    _ = (font_directory / f"Lucide-{version}.txt").write_bytes(license_payload)
    _ = (icon_directory / "lucide_generated.rs").write_text(
        generated_rust, encoding="utf-8", newline="\n"
    )

    print(f"generated {len(icons)} Lucide {version} icons")
    print(f"font BLAKE3: {font_digest}")


def main() -> int:
    """Parse command-line arguments and report generation failures succinctly."""
    parser = argparse.ArgumentParser(description=__doc__)
    _ = parser.add_argument(
        "--version",
        required=True,
        help="lucide-static version to download, such as 1.54.0",
    )
    arguments = parser.parse_args(namespace=Arguments())
    try:
        generate(arguments.version)
    except (
        GenerationError,
        json.JSONDecodeError,
        OSError,
        tarfile.TarError,
        urllib.error.URLError,
    ) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
