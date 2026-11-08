# Third-party notices

Relayix original code is licensed under Apache-2.0. Dependencies remain under their own licenses; the project license does not replace those terms. The image build generates `licenses/`, which includes upstream copyright/license texts and inventories of the locked Rust, installed npm and installed Python distributions used by the Linux build.

- React / React DOM and Vite: MIT. Lucide React: ISC.
- Telethon: MIT. Telegram WEB connector `telethon-webproxy` 0.1.6: MIT, Copyright 2026 fiksofficial. Its license text is included in the generated `licenses/python/`.
- Python transports also include Apache-2.0, MIT, BSD and PSF-2.0 components. Cryptography is dual licensed Apache-2.0 OR BSD-3-Clause; use its Apache-2.0 option. Preserve the separately included licenses of bundled upstream components.
- Rust dependencies are primarily MIT/Apache-2.0, with BSD, ISC, Zlib, Unicode-3.0, CDLA-Permissive-2.0 and other permissive alternatives recorded in the generated `licenses/rust-inventory.json`. When an expression offers MIT/Apache-2.0 alongside LGPL, this distribution uses the MIT/Apache-2.0 alternative. No copyleft-only dependency was found in that inventory.

- The Mattermost SVG logomark is from `webapp/channels/src/components/common/svg_images_components/logo_dark_blue_svg.tsx` in mattermost/mattermost. Copyright Mattermost, Inc. The webapp source is licensed under Apache-2.0. Its original path and color are preserved in the local SVG, together with the origin notice.

Service names and logos identify integrations only. They remain the trademarks of their owners and are not licensed as Relayix trademarks; no endorsement is implied. Other provider pictograms are local illustrative artwork, not additional vendor SDKs. No Telegram Desktop GPL source was copied into Relayix; Telegram transport uses separately licensed Python libraries.

The Docker base operating system and Python interpreter also carry their own licenses and notices. System-package copyright notices remain in `/usr/share/doc` in the image. Relayix notices and collected dependency texts are installed under `/usr/share/doc/relayix`.

Regenerate inventories when dependencies change using `scripts/collect_licenses.py` against `cargo metadata --locked`, the built frontend node_modules and the runtime Python distributions. The inventory records all Cargo metadata packages, including target-specific/build dependencies, so it is broader than the final Linux binary's dependency set.

The selected FakeTLS protocol helpers under `transport/vendor/faketls` are from nsdkinx/tgnet, MIT, Copyright 2026 Dmitry Nasedkin. Their exact upstream commit and import adaptations are recorded in UPSTREAM, and the original MIT license is retained both beside the source and in the generated `licenses/python/`. The Telethon integration and protocol tests are original Relayix code. This vendored helper set is not a PyPI dependency and therefore is listed here separately from the Python package inventory.
