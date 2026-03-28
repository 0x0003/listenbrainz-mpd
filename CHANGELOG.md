# 2.5.0 (2026-03-28)

 - When a file is tagged with multiple artist values, send that list of separate artist names as additional info metadata ([#40](https://codeberg.org/elomatreb/listenbrainz-mpd/issues/40), requested by [alscaldas](https://codeberg.org/alscaldas)).
   This is not yet utilized by the ListenBrainz server, but can be useful with other API-compatible software.
 - When a file is tagged with multiple artists, concatenate the individual values (previously all but the first value would simply be skipped).
 - Dependency updates.

# 2.4.0 (2026-02-27)

- Linux only: Add support for connecting via abstract sockets, prefixed with `@` ([#42](https://codeberg.org/elomatreb/listenbrainz-mpd/pulls/42), by [Kladky](https://codeberg.org/Kladky)).
- Dependency updates.

# 2.3.9 (2025-07-05)

 - Fix some more cases of listens sometimes not being submitted when playing a single track on repeat ([#7](https://codeberg.org/elomatreb/listenbrainz-mpd/issues/7) again, thanks to [Kladky](https://codeberg.org/Kladky)).
 - Fix some tracks where MPD reports a duration of 0 blocking submission to Listenbrainz ([#23](https://codeberg.org/elomatreb/listenbrainz-mpd/issues/23), thanks to [DeeUnderscore](https://codeberg.org/DeeUnderscore)).
 - Dependency updates.

# 2.3.8 (2024-08-11)

 - Allow `--send-feedback` to be passed together with `--config` ([#21](https://codeberg.org/elomatreb/listenbrainz-mpd/issues/21), thanks to [quantenzitrone](https://codeberg.org/quantenzitrone)).
 - Exit with error when the configuration file path provided with `--config` does not exist.
 - Dependency updates.

# 2.3.7 (2024-06-05)

 - Automatically attempt to migrate the submission cache from the deprecated location (see version 2.3.4) to the new default location if it is not explicitly configured ([#20](https://codeberg.org/elomatreb/listenbrainz-mpd/pulls/20), thanks to [Kladky](https://codeberg.org/Kladky)).
 - Dependency updates.

# 2.3.6 (2024-05-19)

 - Fix missing timeout on HTTP API requests potentially resulting submissions getting stuck forever (related to [#19](https://codeberg.org/elomatreb/listenbrainz-mpd/issues/19), thanks to [koraynilay](https://codeberg.org/koraynilay)).
 - Print warning log messages by default when no filter is configured.
 - When built with the `systemd` feature, hide log message timestamps (because journald adds them).
 - Dependency updates.

# 2.3.5 (2024-04-20)

 - Fix configuration files and necessary directories for the config or cache location being created with world-readable permissions.
 - Fix build on Windows.
 - Dependency updates.

# 2.3.4 (2024-04-11)

 - Change default location of submission cache file to better match the XDG spec.

   The new default locations:
   |Platform| Path|
   |-|-|
   |Linux|`$XDG_DATA_HOME/listenbrainz-mpd/submission-cache.sqlite3` or `$HOME/.local/share/listenbrainz-mpd/submission-cache.sqlite3` if `XDG_DATA_HOME` is not set|
   |macOS|`$HOME/Library/Application Support/listenbrainz-mpd/submission-cache.sqlite3`|
   |Windows|`{FOLDERID_LocalAppData}\listenbrainz-mpd\submission-cache.sqlite3`|

   The old default location will continue to be used if no path is explicitly configured and the old path exists.
 - Fix directories for submission cache file location not being created when missing ([#15](https://codeberg.org/elomatreb/listenbrainz-mpd/issues/15), thanks to [GioF71](https://codeberg.org/GioF71)).
 - Fix the explicit `cache_file` config option not doing anything ([#16](https://codeberg.org/elomatreb/listenbrainz-mpd/issues/16), thanks to [GioF71](https://codeberg.org/GioF71)).
 - Dependency updates.

# 2.3.3 (2024-02-07)

 - Fix repeated tracks in certain situations not being counted as separate listens ([#7](https://codeberg.org/elomatreb/listenbrainz-mpd/issues/7), [#14](https://codeberg.org/elomatreb/listenbrainz-mpd/pulls/14), thanks to koraynilay).
 - Internal improvements, dependency updates.

# 2.3.2 (2023-12-17)

 - Add (optional) systemd integration (when built with the `systemd` feature).
   - Service file is now a `Type=notify`.
 - Internal improvements to allow building with different TLS backends.

# 2.3.1 (2023-11-02)

 - Packaging improvements.
   - Add build script that pregenerates shell completion files (behind the `shell_completion` feature).
   - Add man page.

# 2.3.0 (2023-10-31)

 - Support loading key configuration values from environment variables.
   - The configuration file may now be absent if the required values can be loaded from the environment.
   - The ListenBrainz Token can be set using the `LISTENBRAINZ_TOKEN` variable.
     This is the only required configuration value.
   - Support the `MPD_HOST` and `MPD_PORT` environment variables as used by other MPD tools like `mpc`.
     If these are not specified and the configuration file does not configure the MPD address either, the default address `localhost:6600` is assumed.

# 2.2.0 (2023-07-09)

 - Implement ListenBrainz feedback, giving you the ability to mark recordings as "Loved" or "Hated" (#10, requested by oovaga).
   - Accessible by sending messages to the `listenbrainz_feedback` MPD client-to-client channel.
   - As a user, you can use the `mpc` command-line tool like `mpc sendmessage listenbrainz_feedback love`
   - Alternatively, you can use the `--send-feedback` option
 - Fix a bug that prevented a "Now Playing" notification being sent for the first track after starting playback.

# 2.1.0 (2023-03-12)

 - Remember Listens that failed to submit and attempt to submit them again later until they are accepted.
   - Uses an SQLite database as a cache
   - Can be disabled in the configuration if not wanted
 - Add the ability to provide the login token as well as the MPD password as separate files in the configuration (#9, thanks to Scrumplex).
 - The login token is no longer checked immediately on startup.
   - This avoids some issues with the service starting and failing before the network is fully configured
   - Also mitigated by increasing the delay between restart attempts in the provided .service file (#8, thanks to 11xx)
 - No longer exit with an error status if the server closes the connection cleanly.

# 2.0.2 (2023-02-03)

 - Fix listening to the same song twice in a row not generating listen events (#7, thanks to DeeUnderscore).

# 2.0.1 (2023-01-07)

 - Validate MusicBrainz Identifiers before submission, exclude them if invalid (#6, thanks to animakarkia).

# 2.0.0 (2022-08-27)

 - Restructure configuration file. All keys are now grouped into sections.
 - The path to the configuration file can now be overriden with an option (`--config`)  instead of a positional parameter.
