# Dependency license supplements

Some published crates omit their upstream license files. These copies are used
only for the exact package versions listed in `sources.json`; builds fail if a
new version lacks a notice, so an update requires review. Sources for the MIT /
Apache texts are the upstream commits recorded in each crate's
`.cargo_vcs_info.json`. No upstream program code is copied here.

hexf-parse 0.2.1 declares CC0-1.0 in its published Cargo manifest (author Kang
Seonghoon) but supplies no standalone notice. Its supplement is the SPDX License List v3.26.0 copy of the
Creative Commons CC0 1.0 legal text. CC0 does not impose an attribution condition.

The release helper combines these with license/notice files supplied in the
other resolved crate sources, including bundled font notices. This inventory
includes build/test/optional dependencies and is not a claim that all are linked.
Dependency licenses remain separate from Race2Love's PolyForm license.
