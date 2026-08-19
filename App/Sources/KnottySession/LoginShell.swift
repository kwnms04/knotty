import Darwin

/// What a session starts, and how it is asked to start as a login shell.
public enum LoginShell {
    /// The shell the user record names, or the system's own where it names
    /// none — which on macOS is zsh, and has been since Catalina. `/bin/sh` is
    /// the shell a script falls back to, not the one a person was given.
    ///
    /// Read from the record rather than from `SHELL`. An app the window
    /// server started inherits a minimal environment, so what is in it says
    /// nothing about what the user chose — and what `chsh` changed is here.
    public static var path: String {
        guard let record = getpwuid(getuid())?.pointee,
            let named = record.pw_shell,
            let shell = String(validatingCString: named),
            !shell.isEmpty
        else {
            return "/bin/zsh"
        }
        return shell
    }

    /// What to spawn: the shell, and the argument that makes it a login one.
    ///
    /// The conventional spelling is a `-` in front of `argv[0]`, which the
    /// boundary has no way to say — it takes a program and its arguments and
    /// derives `argv[0]` from the program. The argument does the same work,
    /// and every shell knotty can be started with takes it.
    ///
    /// Without it the child gets the minimal environment the app was launched
    /// with, and nothing the user installed is on its path.
    public static var command: [String] { [path, "-l"] }
}
