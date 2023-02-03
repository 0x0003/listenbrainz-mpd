# 2.0.2 (2023-02-03)

 - Fix listening to the same song twice in a row not generating listen events (#7, thanks to DeeUnderscore).

# 2.0.1 (2023-01-07)

 - Validate MusicBrainz Identifiers before submission, exclude them if invalid (#6, thanks to animakarkia).

# 2.0.0 (2022-08-27)

 - Restructure configuration file. All keys are now grouped into sections.
 - The path to the configuration file can now be overriden with an option (`--config`)  instead of a positional parameter.
