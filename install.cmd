@rem Builds macaroni from source, installs it and starts it: see xtask/src/install.rs.
@setlocal
@cd /d "%~dp0"
@cargo xtask install %*
