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
