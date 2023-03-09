# Theoretical design

# Implementation design

BuildXYZ runs a program with a tightly controlled `envp`, namely:

TODO: insert a table of all env vars resetted and their actual paths.

In priority, it will try to read the build environment in tmpfs^[TODO: this is bad for large build environment such as chromium.] and fallback to FUSE filesystem.

## FUSE filesystem

### Basic operations

When the fs receives a lookup request, it is of the form `(parent inode, name)`.

We can resolve the parent inode to some "non prefixed" path, e.g. `parent inode -> bin/` or `parent inode -> include/boost`.

Then, we build the final path `{resolved parent inode path}/{name}`, this is the path we look for in our database of Nix paths using [nix-index](https://github.com/bennofs/nix-index)'s structures.

We may have multiple candidates but we want to assert that all candidates are of the same "kind", e.g. either all symlinks or file *XOR* all symlinks or directories.

Mixing directories and regular files is dangerous because answering to the lookup request with the wrong kind can make legitimate system calls fail.

Now, on to the candidate selection problem.

### Candidate selection

During the lookup call, it is not a big deal to return any candidate (?) as the only call that count is the follow-up readlink that actually ask for the data.

Considering a request for a Boost library, there is a version problem, see: `nix-locate -r include/boost$` as an example.

Also, this example illustrates two more things:

- Development packages containing other development outputs: `bulletml` for example
- Multi-language development package containing native dependency output: `python$VERPackages.boost$VER.dev`.

For BuildXYZ to operate properly, it should be possible to detect the desired version or to operate in some sort of "fuzzing" mechanism where it will record the candidate it has chose for some branching situation and it will try to find out what are the range of possibilities regarding a certain packaging situation.
