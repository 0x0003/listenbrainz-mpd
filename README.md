# `listenbrainz-mpd`

A [ListenBrainz](https://listenbrainz.org) submission client for [MPD](https://www.musicpd.org).

## Features

 - Submission of listen entries, including "Now Playing" entries
 - Full metadata support, including transmission of [MusicBrainz](https://musicbrainz.org) IDs

## Usage

  1. Install the binary.

     #### Cargo

     `cargo install listenbrainz-mpd`


  2. Place the sample configuration file in the appropriate location and fill in your ListenBrainz user token and potentially other relevant configuration.

     | Platform  | Path                                                             |
     | --------- | ---------------------------------------------------------------- |
     | Linux     | `$XDG_CONFIG_HOME/listenbrainz-mpd/config.toml`                  |
     | macOS     | `$HOME/Library/Application Support/listenbrainz-mpd/config.toml` |
     | Windows   | `{FOLDERID_LocalAppData}\listenbrainz-mpd\config.toml`           |

  3. Run the binary, or install the provided [systemd service file](./listenbrainz-mpd.service).


## License

Licensed under the terms of the GNU Affero General Public License v3.0 (see [`LICENSE.txt`](./LICENSE.txt) for details).
