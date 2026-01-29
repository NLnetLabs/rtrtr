# Change Log

## 0.3.3-rc1

Released 2026-01-29.

New

* Changed the version negotiation mechanism of the RTR client to follow
  [draft-ietf-sidrops-8210bis]. (Via [rpki-rs#346])

Bug fixes

* Fixed various aspects of handling of ASPA PDUs:
  * The provider AS set in withdrawal PDUs is now always empty. (Via
    [rpki-rs#350])
  * When updating an ASPA, there will not be a stray withdrawl any more.
    ([#142])

Other changes

* Update the systems binary packages are built for. These are now:
  * Debian Bullseye, Bookworm, and Trixie (ie., 11, 12, 13),
  * Ubuntu Focal 20.04, Jammy 22.04, and Noble 24.04,
  * Red Hat Enterprise Linux 8, 9, and 10 or compatible.

[#142]: https://github.com/NLnetLabs/rtrtr/pull/142
[rpki-rs#346]: https://github.com/NLnetLabs/rpki-rs/pull/346
[rpki-rs#350]: https://github.com/NLnetLabs/rpki-rs/pull/350
[draft-ietf-sidrops-8210bis]: https://datatracker.ietf.org/doc/draft-ietf-sidrops-8210bis/


## 0.3.2 ‘Based on a True Story’

Released 2025-05-06.

There have been no changes since 0.3.2-rc1.


## 0.3.2-rc1

Released 2025-04-24.

New

* Add ASPA support to JSON input and output. ([#132] by [@devsnek])
* ASPA JSON compatibility with krill & routinator ([#134] by [@ember-ana])
* The `json` can now use the native TLS implementations of Windows and
  macOS. This needs to be enabled during compile time through the
  `native-tls` feature and then for a unit through the new `native-tls`
  option. When enabled, this uses TLS via OpenSSL rather than Rustls on
  other systems. ([#137])
* The `json` unit can now be forced to stick to TLS 1.2 or less via the
  new `tls-12` option. ([#137])

Bug fixes

* Fix rtr-tls target having certificate and key options reversed. ([#133]
  by [@ember-ana])
* Fixes to RTR handling via [rpki-rs 0.18.6].
* The `working-dir` option was accidentally used as the path for the PID
  file. Now the `pid-file` option is used as intended. (via
  [daemonbase 0.1.3])

Other changes

* The minimum supported Rust version is now 1.81. ([#138])

[#132]: https://github.com/NLnetLabs/rtrtr/pull/132
[#133]: https://github.com/NLnetLabs/rtrtr/pull/133
[#134]: https://github.com/NLnetLabs/rtrtr/pull/134
[#137]: https://github.com/NLnetLabs/rtrtr/pull/137
[#138]: https://github.com/NLnetLabs/rtrtr/pull/138
[@devsnek]: https://github.com/devsnek
[@ember-ana]: https://github.com/ember-ana
[rpki-rs 0.18.6]: https://github.com/NLnetLabs/rpki-rs/releases/tag/v0.18.6
[daemonbase 0.1.3]: https://github.com/NLnetLabs/daemonbase/releases/tag/v0.1.3



## 0.3.1 ‘Some Checks Haven’t Completed Yet’

Released 2025-01-09.

There have been no changes since 0.3.1-rc3.


## 0.3.1-rc3

Released 2025-01-03.

Bug fixes

* Log a message when a Slurm file fails to load. ([082b7db])

Other changes

* Removed building of packages for Ubuntu 16.04 and 18.04 and Debian
  Stretch and added Ubuntu 24.04.

[#130]: https://github.com/NLnetLabs/rtrtr/pull/130
[082b7db]: https://github.com/NLnetLabs/rtrtr/commit/082b7db8bed80375d3466b4a9ffdf7a818051459

## 0.3.1-rc2

Released 2024-08-14.

New

* Add support for client certificates in the `json` unit. ([#124])

[#124]: https://github.com/NLnetLabs/rtrtr/pull/124


## 0.3.1-rc1

Released 2024-06-19.

Bug Fixes

* Correctly interpret missing `-v` and `-q` options as using the log level
  specified in the config file. (via [daemonbase 0.1.2])

[daemonbase 0.1.2]: https://github.com/NLnetLabs/daemonbase/releases/tag/v0.1.2


## 0.3.0 ‘Filmed Before a Live Studio Audience’

Released 2024-06-06.

There have been no changes since 0.3.0-rc1.


## 0.3.0-rc1

Released 2024-05-29.

Breaking Changes

* Upgrade Rust edition, minimal Rust version to 1.70, and dependencies.
  ([#88], [#91])
* Removed internal serial numbers and the ability to pass optional diffs
  between units. ([#96])

New

* Added a new `merge` unit that merges the datasets of all its sources.
  ([#110], [#113])
* Added four new configuration options to the HTTP client:
  * `http-root-certs` for additional TLS root certificates,
  * `http-user-agent` for setting a custom user agent,
  * `http-client-addr` to specify a local address to bind to, and
  * `http-proxies` to add HTTP proxies (only available if the `socks` feature
     is enabled which it is by default). ([#111])
* The RTR timer values can now be configured for the RTR target. ([#106])
* The RTR target now produces metrics. By setting `client-metrics: true`
  in its configuration, the target produces separate metrics for each
  client address. ([#115], [#117])
* Log changes made by the `slurm` unit to updates. ([#87])
* The `slurm` unit now updates its data set if it discovers that the Slurm
  files have changed. ([#89])
* Both the `json` unit and target now support conditional HTTP requests
  via the Etag and Last-Modified headers. ([#98])

Bug Fixes

* Fix a race condition where the `slurm` unit would not apply its changes
  to the first update if loading the files is too slow. ([#89])
* Fixed various race conditions during startup and shutdown. ([#101])

Other Changes

* Upgrade the packaging and Docker build workflow to allow for
  cross-compilation. ([#90])

[#87]: https://github.com/NLnetLabs/rtrtr/pull/87
[#88]: https://github.com/NLnetLabs/rtrtr/pull/88
[#89]: https://github.com/NLnetLabs/rtrtr/pull/89
[#90]: https://github.com/NLnetLabs/rtrtr/pull/90
[#91]: https://github.com/NLnetLabs/rtrtr/pull/91
[#96]: https://github.com/NLnetLabs/rtrtr/pull/96
[#98]: https://github.com/NLnetLabs/rtrtr/pull/98
[#101]: https://github.com/NLnetLabs/rtrtr/pull/101
[#106]: https://github.com/NLnetLabs/rtrtr/pull/106
[#110]: https://github.com/NLnetLabs/rtrtr/pull/110
[#111]: https://github.com/NLnetLabs/rtrtr/pull/111
[#113]: https://github.com/NLnetLabs/rtrtr/pull/113
[#115]: https://github.com/NLnetLabs/rtrtr/pull/115
[#117]: https://github.com/NLnetLabs/rtrtr/pull/117


## 0.2.2

Released 2022-07-13.

Bug Fixes

* Fix a formatting bug in JSON output that caused ASNs to be prefixed with
  `ASAS`. ([#77])

[#77]: https://github.com/NLnetLabs/rtrtr/pull/77


## 0.2.2-rc1

Released 2022-06-02.

Bug Fixes

* Fix a bug that cause targets to produce duplicate items in their output.
  ([#73])
* Fix a formatting bug in JSON output that caused the prefix length to
  appear twice. ([#74])

Other Changes

* Added support for packaging for Ubuntu 22.04 Jammy Jellyfish. ([#70])

[#70]: https://github.com/NLnetLabs/rtrtr/pull/70
[#73]: https://github.com/NLnetLabs/rtrtr/pull/73
[#74]: https://github.com/NLnetLabs/rtrtr/pull/74


## 0.2.1

Released 2022-03-28.

There have been no changes since 0.2.1-rc1.


## 0.2.1-rc1

Released 2022-03-16.

Bug Fixes

* Fixed an issue that resulted in the `"rtr"` and `"rtr-tls"` targets
  keeping an endlessly growing list of diffs and continuously increasing
  memory consumption. ([#65])

New

* The number of diffs kept by the `"rtr"` and `"rtr-tls"` units can now
  be configured via the new `"history-size"` config option. This new
  option is optional and defaults to 10. ([#65])

[#65]: https://github.com/NLnetLabs/rtrtr/pull/65


## 0.2.0 ‘Arts and Crafts and Tactical Gear’

Released 2022-01-19.

There have been no changes since 0.2.0-rc1.


## 0.2.0-rc1

Released 2022-01-12.

Breaking Changes

* The minimum supported Rust version is now 1.54. ([#45])

New

* Relative paths in config files are now resolved relative to the
  directory the config file is stored in. ([#49], [#50])
* The `"json"` unit now supports the modified JSON format used by newer
  versions of rpki-client. That is, it accepts ASNs as numbers or
  strings and ignores any fields that aren’t essential. ([#30], [#32])
* Added a `"slurm"` unit that can be used to manipulate payload sets based
  on local exception files defined in [RFC 8416]. ([#31])
* Added `"rtr-tls"` unit and target that send RTR data over TLS
  connections. ([#34])
* New metrics for the `"rtr"` and `"rtr-tls"` units list the session ID,
  serial number, and time of the last update, as well as total number of
  bytes read from and sent to the server. ([#40])
* Metrics are now sorted alphabetically (with a few exceptions) in output.
  ([#53])

Bug Fixes

* Corrected the RTR PDU type of the Cache Reset PDU from 7 to 8.
  ([rpki #151])
* The `--config` command line option is now mandatory, resulting in a
  proper error message when it is missing rather than a panic. ([#41])
* The `"json"` unit will not trigger an update if the data source hasn’t
  changed. ([#51])

Other

* Upgraded to Tokio 1.0, Hyper 0.14, and Reqwest 0.11. ([#17]) 

[#30]: https://github.com/NLnetLabs/rtrtr/pull/30
[#31]: https://github.com/NLnetLabs/rtrtr/pull/31
[#32]: https://github.com/NLnetLabs/rtrtr/pull/32
[#34]: https://github.com/NLnetLabs/rtrtr/pull/34
[#40]: https://github.com/NLnetLabs/rtrtr/pull/40
[#41]: https://github.com/NLnetLabs/rtrtr/pull/41
[#45]: https://github.com/NLnetLabs/rtrtr/pull/45
[#49]: https://github.com/NLnetLabs/rtrtr/pull/49
[#50]: https://github.com/NLnetLabs/rtrtr/pull/50
[#51]: https://github.com/NLnetLabs/rtrtr/pull/51
[#53]: https://github.com/NLnetLabs/rtrtr/pull/53
[rpki #151]: https://github.com/NLnetLabs/rpki-rs/pull/151
[RFC 8416]: https://tools.ietf.org/html/rfc8416


## 0.1.2 ‘Ten Four’

Released 2021-03-15

New

* The JSON unit ignores the `metadata` field in received files. This
  makes it compatible with the JSON produced by at least Routinator, OctoRPKI,
  and rpki-client. ([#8])


[#8]: https://github.com/NLnetLabs/rtrtr/pull/8


## 0.1.1 ‘Death Metal Karaoke’

Released 2020-12-11

New

* Support for JSON via HTTP and from a local file as a source, and JSON
  via HTTP as a target. ([#5])

[#5]: https://github.com/NLnetLabs/rtrtr/pull/5


## 0.1.0 ‘Little Ball of Fur’

Released 2020-11-09.

Initial public release.

