@echo off
title Mohona - cargo test
cargo test --workspace

title Mohona - cargo build
cargo build --release -p mohona-cli -p mohona-host

title Mohona - installing host
target\release\mohona-host install

title Mohona - extension build
cd extension

npm install

npm run check

cd..

title Mohona - app build
cd app

npm install

$env:MOHONA_PROGRAM = "D:\990\Personal\@Development\Rust\mohona\target\release\mohona.exe"

npm run tauri dev