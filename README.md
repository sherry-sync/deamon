# Sherry demon

This repository includes implementation of sherry demon.
The app watches sherry directories to collect file system events and sync them with server

## Build

```bash
cargo build --release
```

The built file will be `./target/release/sherry-demon[.exe]`.

## Usage

```bash
sherry-demon [--config "<CONFIG PATH>"]
```

## Development & Testing

On start, the app tries to create config directory with all required state in `~/.sherry` (User's home directory).
To overwrite this behavior additional param can be specified: `--config "<CONFIG PATH>"`.
It is not required, but can be used for development.

This folder contains the whole sherry state, so deleting it will remove all authorizations and directory synchronisations.
> Actually really can be used to wipe all the app state 🤔.

For development, it is recommended to use local configuration for easy access to it. 
`dev-config` folder already added to `.gitignore`, so it is recommended to use this command for development:

```bash
cargo run -- -c ./dev-config
```

`<CONFIG PATH>/logs` will contain app logs.
