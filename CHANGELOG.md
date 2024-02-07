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
