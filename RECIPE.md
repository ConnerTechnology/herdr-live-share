# herdr live share: install recipe

For the agent that was asked to "add this mod: https://github.com/ConnerTechnology/herdr-live-share". Read this whole file before you run anything.

## What gets installed, and where

herdr live share has two parts, and they often belong on different computers:

- **Five mods** (`share-settings`, `share-start`, `share-join`, `share-activity`, `share-updates`). They go into `~/.config/herdr-app/mods/` on **the computer running herdr-app**, the one the person sits at.
- **The engine** (`herdr-session`, one binary). It goes into `~/.local/bin` on **every machine whose spaces are shared**: the machines that hold the repos, run git and run Claude. That is often a remote machine shown in the sidebar under its own name, not "This computer".

Both are installed by one script, `install.sh`. It works out which computer it runs on and does the right part.

## Steps

1. **Find out where you are running.** Run `herdr-app status`.
   - It succeeds: you are on the herdr-app computer. Go to step 2.
   - It fails with "no Herdr window": you are on a remote machine (the person's space is on another computer). Go to step 3.

2. **On the herdr-app computer**, run:

   ```sh
   curl -fsSL https://raw.githubusercontent.com/ConnerTechnology/herdr-live-share/main/install.sh | sh
   ```

   It installs the engine here, copies the five mods into `~/.config/herdr-app/mods/`, then installs the engine on every connected machine in `hc machine ls` through `hc exec --on <machine>`. A machine that fails is named in the output with the command to run there. Go to step 4.

3. **On a remote machine**, the herdr-app computer can't be reached from here. Run the same command:

   ```sh
   curl -fsSL https://raw.githubusercontent.com/ConnerTechnology/herdr-live-share/main/install.sh | sh
   ```

   It installs the engine on this machine and prints what to do on the herdr-app computer. Tell the person, in these words: "The engine is installed on <this machine>. To finish, paste the same line into an agent in a space under This computer, or run `curl -fsSL https://raw.githubusercontent.com/ConnerTechnology/herdr-live-share/main/install.sh | sh` in a terminal there." Stop here.

4. **Check.** For each mod, run `herdr-app check ~/.config/herdr-app/mods/<mod>`. Then run `hc exec --on <machine> herdr-session version` for each machine from step 2 (leave out `--on` for this computer). Each should print `"ok":true`.

## Watch out

- **Don't change the person's shell PATH.** herdr-core runs programs with its own PATH, which includes `~/.local/bin`, so that is where the engine goes. `~/.cargo/bin` is not on it.
- **Rust is needed only when no release binary fits the machine.** If the script says Rust is missing, tell the person. Don't install Rust without asking.
- **Mod folders are fixed names.** If a `share-*` folder is a symlink, it is a developer checkout: the script leaves it alone, and so should you. If one exists and isn't from this repo, stop and ask.
- **Nothing else is changed.** The script does not touch layouts, keys, `settings.json` or `~/.claude/settings.json`. Claude hooks are added later, by the engine, when the person starts or joins a shared space.
- A machine added later gets the engine from the dialogs: when a share dialog finds it missing, it offers **Install on <machine>**.

## Done when

- `herdr-app check` passes for all five mods on the herdr-app computer.
- `herdr-session version` prints `"ok":true` on every machine with spaces to share.
- The command palette in herdr-app lists **Share this space**, **Join a space** and **Live share updates**.
