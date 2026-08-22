# bevy_hana

Public source mirror for the Bevy crates published from the private
`natepiano/hana` workspace.

## What this is

Every crate here is published to crates.io from the private workspace. This
repository exists so their **examples and tests can be seen to build against
their real dependencies** -- something a crates.io tarball cannot show you,
because path-only dev-dependencies are stripped when a crate is packaged.

The unpublished crates in `crates/` are carried as source for exactly that
reason. They are not published and are not intended for direct use.

## What this is not

**Generated. Do not send pull requests here.** The tree is produced by
`scripts/mirror.py` in the private workspace and force-synced on each release;
anything committed directly is overwritten. Issues are welcome -- fixes land
upstream and arrive here on the next release.

## Published crates

| Crate | crates.io |
|---|---|
| [`bevy_kana`](crates/bevy_kana) | [![crates.io](https://img.shields.io/crates/v/bevy_kana.svg)](https://crates.io/crates/bevy_kana) |
| [`hana_clerestory`](crates/hana_clerestory) | [![crates.io](https://img.shields.io/crates/v/hana_clerestory.svg)](https://crates.io/crates/hana_clerestory) |
| [`hana_lagrange`](crates/hana_lagrange) | [![crates.io](https://img.shields.io/crates/v/hana_lagrange.svg)](https://crates.io/crates/hana_lagrange) |
| [`hana_liminal`](crates/hana_liminal) | [![crates.io](https://img.shields.io/crates/v/hana_liminal.svg)](https://crates.io/crates/hana_liminal) |
| [`hana_rigging`](crates/hana_rigging) | [![crates.io](https://img.shields.io/crates/v/hana_rigging.svg)](https://crates.io/crates/hana_rigging) |
| [`hana_rubric`](crates/hana_rubric) | [![crates.io](https://img.shields.io/crates/v/hana_rubric.svg)](https://crates.io/crates/hana_rubric) |

## Carried as source

- [`fairy_dust`](crates/fairy_dust)
- [`hana_diegetic`](crates/hana_diegetic)
- [`hana_rigging_scripted`](crates/hana_rigging_scripted)
- [`hana_valence`](crates/hana_valence)

## License

Dual-licensed under MIT or Apache-2.0, per crate.
