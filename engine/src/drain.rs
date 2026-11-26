//! Detached child (setsid) spawned by the Stop hook: pull --rebase --autostash,
//! push the branch, append Pushed/Conflict/BranchRewritten, push own stream; exits.
//! Spike: a setsid child outlives a hook that exits normally.
