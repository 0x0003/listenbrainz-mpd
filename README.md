# `listenbrainz-mpd`

A [ListenBrainz](https://listenbrainz.org) submission client for [MPD](https://www.musicpd.org).

## Features

 - Submission of listen entries, including "Now Playing" entries
 - Full metadata support, including transmission of [MusicBrainz](https://musicbrainz.org) IDs
 - Ability to submit feedback (Love/Hate) for individual recordings

## Usage

  1. Install using one of the methods below.

     #### Package Managers

     This software is packaged by some Linux distributions in their native repositories.
     The following is a non-exhaustive list, feel free to open an issue to add more.
     Note that the package definitions are maintained by third parties.

     | Distribution | Package name |
     | ------------ | ------------ |
     | Arch Linux   | [`listenbrainz-mpd`](https://archlinux.org/packages/extra/x86_64/listenbrainz-mpd/) |
     | Nix          | [`listenbrainz-mpd`](https://search.nixos.org/packages?channel=unstable&query=listenbrainz-mpd) |

     #### Cargo

     Run `cargo install --locked listenbrainz-mpd`.
     This will build from source and may take a non-trivial amount of time and resources on weak systems.

     > [!NOTE]
     > Check the [Building From Source](#building-from-source) section for additional steps you may need to take.

     #### Other Options

     - Docker: [GioF71/listenbrainz-mpd-docker](https://github.com/GioF71/listenbrainz-mpd-docker)

  2. Configure your ListenBrainz user token and other preferences.

     You can obtain your token [here](https://listenbrainz.org/settings/).

     Place the [sample configuration file](./config.toml.sample) in the appropriate location and fill in your user token and potentially other relevant configuration.
     You can use the `--create-default-config` option to have this file automatically created for you.

     | Platform  | Default config file location                                     |
     | --------- | ---------------------------------------------------------------- |
     | Linux     | `$XDG_CONFIG_HOME/listenbrainz-mpd/config.toml`                  |
     | macOS     | `$HOME/Library/Application Support/listenbrainz-mpd/config.toml` |
     | Windows   | `{FOLDERID_LocalAppData}\listenbrainz-mpd\config.toml`           |

     Some configuration details can also be specified via environment variables, potentially making a config file unnecessary.
     See the [manual page](./listenbrainz-mpd.adoc) for details.

  3. Run the binary.

     How exactly you want to do this depends on your setup, but typically you want a systemd user service.

     If you installed the software from your OS package manager, this is usually already prepared for you.
     If you built from source, a [sample service definition](./listenbrainz-mpd.service) is provided.

     > [!NOTE]
     > The sample service file assumes the binary was built with the `systemd` feature flag.

     Once the systemd service is installed, you can start it with the following command:

     `systemctl --user enable --now listenbrainz-mpd.service`

## Building From Source

You can build from source using standard Rust tooling, i.e. `cargo build --release`.

### Required Native Dependencies

Headers for the [native SQLite library](https://crates.io/crates/libsqlite3-sys) as well as your platforms [native TLS library](https://crates.io/crates/native-tls) are required.

On Linux, these typically need to be installed using your **distribution package manager**.
As an example, for Debian you would need the `libsqlite3-dev` and `libssl-dev` header packages for a successful compilation, as well as their runtime equivalents (`sqlite3` and `openssl` respectively) to actually run the binary.
The package names may differ for other distributions.

### Optional Features

Some additional functionality can be enabled via [Cargo features](https://doc.rust-lang.org/cargo/reference/features.html).
To enable a feature, use the `--feature example_feature` Cargo flag.

#### `shell_completion`

Generate completion definitions for a variety of interactive shells.
The generated files will be placed into a directory specified by the `COMPLETIONS_OUT_DIR` environment variable (at build time), or `./generated_completions` if this is not set.

#### `systemd`

Enable systemd integration for the process, including emitting readiness and status notifications for use with `Type=notify` services, and removing timestamps from any log messages (since they are automatically recorded by systemd-journald).

The provided [systemd service file](./listenbrainz-mpd.service) requires the binary to be built with the `systemd` feature, since it relies on readiness notifications.

## License

Licensed under the terms of the GNU Affero General Public License v3.0 (see [`LICENSE.txt`](./LICENSE.txt) for details).
