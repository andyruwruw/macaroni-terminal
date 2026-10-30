# ConPTY

`conpty.dll` and `OpenConsole.exe` from the [Microsoft.Windows.Console.ConPTY](https://www.nuget.org/packages/Microsoft.Windows.Console.ConPTY) NuGet package, version **1.25.260930003**, MIT licensed ([microsoft/terminal](https://github.com/microsoft/terminal)).

`portable-pty` loads `conpty.dll` by bare name, so placing both files next to the executable makes it use this ConPTY instead of the older one built into Windows. `crates/app/build.rs` copies them into the Cargo target directory on Windows builds.

To update: download the `.nupkg` (a zip) and copy `runtimes/win-<arch>/native/conpty.dll` and `build/native/runtimes/<arch>/OpenConsole.exe` for `x64` and `arm64`.
