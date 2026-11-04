@echo off
title Compile and Build Mohona

cargo test --workspace

cargo build --release -p mohona-cli -p mohona-host

target\release\mohona-host install

cd extension

npm install

npm run check

cd..

cd app

npm install

$env:MOHONA_PROGRAM = "D:\990\Personal\@Development\Rust\mohona\target\release\mohona.exe"

npm run tauri dev