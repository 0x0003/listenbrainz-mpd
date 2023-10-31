# `listenbrainz-mpd`

A [ListenBrainz](https://listenbrainz.org) submission client for [MPD](https://www.musicpd.org).

## Features

 - Submission of listen entries, including "Now Playing" entries
 - Full metadata support, including transmission of [MusicBrainz](https://musicbrainz.org) IDs
 - Ability to submit feedback (Love/Hate) for individual recordings

## Usage

  1. Install.

     #### AUR (Arch Linux)

     Install the [`listenbrainz-mpd`](https://aur.archlinux.org/packages/listenbrainz-mpd) AUR package.

     #### Cargo

     `cargo install listenbrainz-mpd`

  2. Configure your ListenBrainz user token through the configuration file or the `LISTENBRAINZ_TOKEN` environment variable.

     Place the [sample configuration file](./config.toml.sample) in the appropriate location and fill in your ListenBrainz user token and potentially other relevant configuration.

     | Platform  | Default config file location                                     |
     | --------- | ---------------------------------------------------------------- |
     | Linux     | `$XDG_CONFIG_HOME/listenbrainz-mpd/config.toml`                  |
     | macOS     | `$HOME/Library/Application Support/listenbrainz-mpd/config.toml` |
     | Windows   | `{FOLDERID_LocalAppData}\listenbrainz-mpd\config.toml`           |

     You can use the `--create-default-config` option to have this file automatically created for you.

  3. Run the binary, or install and enable the provided [systemd service file](./listenbrainz-mpd.service).

     `systemctl --user enable --now listenbrainz-mpd.service`


## License

Licensed under the terms of the GNU Affero General Public License v3.0 (see [`LICENSE.txt`](./LICENSE.txt) for details).
