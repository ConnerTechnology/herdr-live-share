// Join a space. Paper: "Join 2 · Enter the share link", "Join 3 · Ryan opens
// the link", "Join 3 · Step N fails", "Join 4 · Joined", "Join errors · each step".
//
// Paste a link, then five engine steps run one by one (`join --step …`), each
// re-checking the ones before, so Try again and a reopened dialog are safe.
// The last step makes the worktree; the space is added to the sidebar without
// showing it or opening Claude.
import { View, div, svg } from "gpui-kit";
import { v_flex, h_flex, InputState, Input, Popover } from "gpui-base";
import { props, closeOverlay, theme, checking, run, spaces, panes, machines, shared, setShared } from "herdr";
import { engine, installEngine, newer, shownSpace, machineName, VERSION } from "./engine.js";

const STEPS = ["session", "github", "access", "folder", "branch"];
const LONG = 600_000; // the first checkout can take minutes
const DEFAULT_FOLDER = "~/herdr-sessions";

/** herdr's own icons, shipped with its workspace-chrome mod. */
const ICON = (name) => `mods/workspace-chrome/${name}.svg`;

/** A machine's name as a menu item: "This computer", or its name. */
const placeLabel = (on) => on ?? "This computer";

/** Where a join can go: this computer, then every connected machine. */
function places() {
  return [null, ...machines().filter((m) => m.layout !== "unavailable" && m.name !== null).map((m) => m.name)];
}

/** Which row an engine error belongs to (step 1 also reports 2 and 3: GitHub hides private repos). */
const ROW = { BadLink: 0, SessionNotFound: 0, Closed: 0, Network: 0, NotSignedIn: 1, GhMissing: 1, NoAccess: 2, NotInvited: 2, FolderUnusable: 3, BranchGone: 4 };

/** Where a space is now: its first terminal's live folder, else its own. */
function place(s) {
  const tabs = new Set((s.tabs ?? []).map((c) => c.id));
  return panes().find((p) => tabs.has(p.tab) && p.cwd)?.cwd || s.folder;
}

/** ~/herdr-sessions/…/google-search */
function short(path) {
  const parts = String(path).split("/");
  return parts.length > 3 ? `${parts[0]}/${parts[1]}/…/${parts[parts.length - 1]}` : parts.join("/");
}

export default class ShareJoin extends View {
  init(_p, cx) {
    this.handle = cx.focus_handle();
    this.on = shownSpace()?.machine ?? null;
    this.state = "link"; // "link" | "steps"
    this.rows = STEPS.map(() => "pending"); // pending | running | pass | fail
    this.facts = null; // the engine's reply: owner, name, repo, branch, events, login, permission, worktree
    this.failure = null; // { kind, message, row }
    this.engineNote = null;
    this.fix = null;
    this.busy = false;
    this.machineOpen = false;
    const start = props()?.link ?? shared("share.joinLink") ?? "";
    this.link = InputState.new({ value: start, placeholder: "herdr.dev/join/owner/repo/code" });
    this.link.on("submit", (_e, cx) => this.join(cx));
    this.link.on("change", (_e, cx) => cx.notify());
    this.folder = InputState.new({ value: this.savedFolder() ?? DEFAULT_FOLDER });
    this.folder.on("submit", (_e, cx) => this.join(cx));
    // The link field takes the keyboard (again once the overlay has settled, as the palette does).
    this.link.focus();
    cx.timer.after(600, () => this.link.focus());
    if (checking()) return;
    cx.spawn(async (cx) => {
      await this.checkEngine(cx);
      if (props()?.link) this.join(cx); // opened from a link: go straight to the steps
    });
  }

  savedFolder() {
    return shared("share.joinFolders")?.[this.on ?? ""] ?? null;
  }

  /** The engine on the chosen machine: present and at least as new as these mods. */
  async checkEngine(cx) {
    const r = await engine(["version"], { on: this.on });
    this.fix = null;
    this.engineNote = null;
    if (!r.ok) {
      this.fix = r.error.kind === "Missing" ? "install" : null;
      this.engineNote = r.error.message;
    } else if (newer(VERSION, r.value.version)) {
      this.fix = "update";
      this.engineNote = `herdr-session ${r.value.version} on ${machineName(this.on)} is older than these mods (${VERSION}).`;
    }
    cx.notify();
  }

  installHere(cx) {
    this.busy = true;
    this.engineNote = `Installing herdr-session on ${machineName(this.on)}…`;
    cx.notify();
    cx.spawn(async (cx) => {
      const r = await installEngine(this.on);
      this.busy = false;
      if (r.ok) await this.checkEngine(cx);
      else { this.engineNote = r.error.message; cx.notify(); }
    });
  }

  /** The machine the join goes to (O13); its own saved folder and engine come with it. */
  chooseMachine(cx, on) {
    this.machineOpen = false;
    if (on !== this.on) {
      this.on = on;
      this.folder.set_value(this.savedFolder() ?? DEFAULT_FOLDER);
      this.engineNote = null;
      this.fix = null;
      cx.spawn(async (cx) => this.checkEngine(cx));
    }
    cx.notify();
  }

  join(cx) {
    const link = this.link.value().trim();
    if (!link || this.running || this.engineNote) return;
    setShared("share.joinLink", link);
    const folder = this.folder.value().trim() || DEFAULT_FOLDER;
    if (folder !== DEFAULT_FOLDER || this.savedFolder()) {
      setShared("share.joinFolders", { ...(shared("share.joinFolders") ?? {}), [this.on ?? ""]: folder });
    }
    this.state = "steps";
    this.failure = null;
    this.rows = STEPS.map(() => "pending");
    this.running = true;
    cx.notify();
    cx.spawn(async (cx) => {
      for (let i = 0; i < STEPS.length; i++) {
        this.rows[i] = "running";
        cx.notify();
        const args = ["join", "--link", link, "--step", STEPS[i]];
        if (i >= 3) args.push("--folder", folder);
        const r = await engine(args, { on: this.on, timeout: i === 4 ? LONG : 120_000 });
        if (!r.ok) {
          const row = Math.max(ROW[r.error.kind] ?? i, i);
          // Rows between this step and the failing one stay unknown (grey), never green.
          for (let j = i; j < STEPS.length; j++) this.rows[j] = j === row ? "fail" : "pending";
          this.failure = { ...r.error, row };
          this.running = false;
          cx.notify();
          return;
        }
        this.facts = r.value;
        this.rows[i] = "pass";
        cx.notify();
      }
      this.running = false;
      setShared("share.joinLink", null);
      this.addSpace();
      cx.notify();
    });
  }

  /** The joined worktree as a space: found by its folder, else made, not shown. */
  addSpace() {
    const f = this.facts;
    if (!f?.worktree) return;
    const there = spaces().find((s) => (s.machine ?? null) === (this.on ?? null) && place(s) === f.worktree);
    if (there) return;
    run("space.new", { name: f.name, folder: f.worktree, machine: this.on ?? "", focus: false });
  }

  /** Not signed in: `gh auth login` in a tab on the machine; the link waits for the dialog's return. */
  signIn() {
    run("tab.new", { name: "GitHub sign-in", argv: ["gh", "auth", "login", "--web"], ...(this.on ? { space: shownSpace()?.id } : {}) });
    this.close();
  }

  close() {
    closeOverlay(props()?.overlay);
  }

  // ---------------------------------------------------------------- drawing

  render(cx) {
    const t = theme();
    const green = t.ansi2 ?? t.done;
    const red = t.ansi1 ?? t.blocked;
    const primary = (id, label, enabled, onClick) =>
      h_flex().id(id).h(34).px(16).items_center().rounded(8).bg(t.accent)
        .text_size(13).font_semibold().text_color(t.accent_foreground)
        .when(!enabled, (el) => el.opacity(0.5))
        .when(enabled, (el) => el.cursor_pointer().hover((st) => st.opacity(0.88)).on_click(onClick))
        .child(label);
    const cancel = div().id("cancel").text_size(13).text_color(t.muted).underline().cursor_pointer()
      .hover((st) => st.text_color(t.foreground)).on_click(() => this.close()).child("Cancel");
    const field = (label, state, bad) =>
      v_flex().gap(6)
        .child(div().text_size(11).font_semibold().text_color(t.dim).child(label))
        .child(Input.new(state).h(38).px(12).rounded(8).border_1().border_color(bad ? red : t.border).bg(t.background)
          .font_family("JetBrains Mono").text_size(13).text_color(t.foreground));

    // A labelled dropdown, drawn like the Who can join menu, when there is more than one place.
    const options = places();
    const placeIcon = (on) => svg(ICON(on === null ? "monitor" : "server")).size(14).flex_none().text_color(t.muted);
    const machineRow = options.length > 1
      ? v_flex().gap(6)
        .child(div().text_size(11).font_semibold().text_color(t.dim).child("JOIN ON"))
        .child(Popover.new("machine-menu").anchor("top_left")
          .open(this.machineOpen && !this.busy)
          .on_open_change((open, cx) => { this.machineOpen = open; cx.notify(); })
          .trigger(h_flex().id("machine").w_full().h(38).px(12).gap(8).items_center().rounded(8).border_1().border_color(t.border)
            .bg(this.machineOpen ? t.hover : t.background).cursor_pointer().hover((st) => st.bg(t.hover))
            .child(placeIcon(this.on))
            .child(div().flex_1().min_w(0).whitespace_nowrap().overflow_hidden().text_ellipsis().text_size(13).text_color(t.foreground).child(placeLabel(this.on)))
            .child(svg(ICON("chevron-down")).size(14).flex_none().text_color(t.muted)))
          .content(v_flex().id("machine-list").w(512).mt(6).p(5).rounded(10).border_1().border_color(t.border).bg(t.popover).shadow_lg()
            .children(options.map((on) => h_flex().id(`machine-${on ?? "here"}`).h(32).px(8).gap(9).rounded(7).items_center().cursor_pointer()
              .hover((st) => st.bg(t.hover)).on_click((_e, cx) => this.chooseMachine(cx, on))
              .child(placeIcon(on))
              .child(div().flex_1().min_w(0).whitespace_nowrap().overflow_hidden().text_ellipsis().text_size(13).text_color(t.foreground).child(placeLabel(on)))
              .child(div().w(14).flex_none().when(on === this.on, (el) => el.child(svg(ICON("check")).size(14).text_color(t.accent))))))))
      : null;
    const engineRow = this.engineNote
      ? h_flex().gap(12).items_center()
        .child(div().flex_1().min_w(0).text_size(12).text_color(t.muted).child(this.engineNote))
        .when(Boolean(this.fix) && !this.busy, (el) => el.child(
          h_flex().id("fix").h(30).px(12).items_center().rounded(8).border_1().border_color(t.border).text_size(12)
            .text_color(t.foreground).cursor_pointer().on_click((_e, cx) => this.installHere(cx))
            .child(`${this.fix === "install" ? "Install" : "Update"} on ${machineName(this.on)}`)))
      : null;

    const body = this.state === "link" ? this.renderLink(t, field, machineRow, engineRow, cancel, primary) : this.renderSteps(cx, t, green, red, field, cancel, primary, engineRow);
    return v_flex()
      .id("share-join")
      .track_focus(this.handle)
      .on_key_down((e, cx) => {
        if (e.key !== "escape") return;
        this.close();
        cx.stop_propagation();
      })
      .size_full()
      .p(24)
      .gap(20)
      .rounded(12)
      .bg(t.popover)
      .child(body);
  }

  renderLink(t, field, machineRow, engineRow, cancel, primary) {
    const ready = this.link.value().trim().length > 0 && !this.engineNote;
    return v_flex().gap(20)
      .child(div().self_center().text_size(17).font_semibold().text_color(t.foreground).child("Join a shared space"))
      .child(v_flex().gap(16).child(field("SHARE LINK", this.link, false)).when(Boolean(machineRow), (el) => el.child(machineRow)))
      .when(Boolean(engineRow), (el) => el.child(engineRow))
      .child(h_flex().items_center().justify_between()
        .child(cancel)
        .child(primary("join", "Join session", ready, (_e, cx) => this.join(cx))));
  }

  renderSteps(cx, t, green, red, field, cancel, primary, engineRow) {
    const f = this.facts;
    const branch = f?.branch;
    const labels = {
      pending: ["Find the session on GitHub", "Sign in to GitHub", "Check your access to the repository", "Create your worktree", branch ? `Check out ${branch}` : "Check out the branch"],
      running: ["Finding the session on GitHub", "Checking your GitHub sign-in", "Checking your access to the repository", "Creating your worktree", branch ? `Checking out ${branch}` : "Checking out the branch"],
      pass: ["Found the session on GitHub", `Signed in to GitHub as ${f?.login ?? ""}`, `You have ${f?.permission ?? "write"} access to the repository`, "Created your worktree", `Checked out ${branch ?? "the branch"}`],
    };
    const failLabel = (k) => ({
      BadLink: "That isn't a share link", SessionNotFound: "Session not found", Closed: "This session is closed", Network: "Couldn't reach GitHub",
      NotSignedIn: "Not signed in to GitHub", GhMissing: "The GitHub CLI isn't installed", NoAccess: "No access to this repository",
      NotInvited: "Not on the selected list", FolderUnusable: "Couldn't create your worktree", BranchGone: "The branch is gone from GitHub",
    })[k] ?? "Something went wrong";
    const detail = (i, st) => {
      if (st === "pass" && i === 0) return f ? `${f.events} ${f.events === 1 ? "event" : "events"}` : "";
      if (st === "pass" && i === 1) return "from your gh login";
      if (st === "pass" && i === 2) return "checked on GitHub";
      if ((st === "pass" || st === "running") && i === 3 && f?.worktree_shown) return short(f.worktree_shown);
      if (st === "fail" && (i === 0 || i === 2)) return "checked on GitHub";
      return "";
    };
    const dot = (st) => {
      const base = div().w(16).h(16).flex_shrink_0().rounded(8).flex().items_center().justify_center().text_size(10).font_semibold();
      if (st === "pass") return base.bg(green).text_color(t.background).child("✓");
      if (st === "fail") return base.bg(red).text_color(t.background).child("×");
      if (st === "running") return base.border_2().border_color(t.accent);
      return base.border_1().border_color(t.border);
    };
    const rows = v_flex().children(STEPS.map((_, i) => {
      const st = this.rows[i];
      const text = st === "fail" ? failLabel(this.failure?.kind) : labels[st === "running" ? "running" : st === "pass" ? "pass" : "pending"][i];
      return h_flex().id(`row-${i}`).gap(12).items_center().py(9).when(i < STEPS.length - 1, (el) => el.border_b_1().border_color(t.border))
        .child(dot(st))
        .child(div().flex_1().min_w(0).text_size(13)
          .text_color(st === "fail" ? red : st === "pending" ? t.dim : t.foreground)
          .when(st === "running" || st === "fail", (el) => el.font_semibold())
          .child(text))
        .child(div().flex_shrink_0().text_size(12).text_color(t.dim).when(i === 3, (el) => el.font_family("JetBrains Mono")).child(detail(i, st)));
    }));

    const fail = this.failure;
    const box = (title, text) => v_flex().gap(4).p(12).rounded(8).border_1().border_color(red).bg(t.background)
      .child(div().text_size(13).font_semibold().text_color(red).child(title))
      .child(div().text_size(12).text_color(t.muted).child(text));
    let fixArea = null;
    let action = primary("done", "Done", !this.running && this.rows.every((r) => r === "pass"), () => this.close());
    if (fail) {
      const retry = primary("retry", "Try again", true, (_e, cx) => this.join(cx));
      action = retry;
      if (fail.row === 0) {
        fixArea = v_flex().gap(8).child(field("SHARE LINK", this.link, true))
          .child(div().text_size(12).text_color(red).child(fail.kind === "SessionNotFound" ? "We couldn't find a session at this link. It may be mistyped, or the owner closed it." : fail.message));
      } else if (fail.kind === "NotSignedIn") {
        fixArea = box("Sign in to continue", "herdr needs your GitHub login to check your access to the repository.");
        action = primary("sign-in", "Sign in to GitHub", true, () => this.signIn());
      } else if (fail.row === 3) {
        fixArea = v_flex().gap(8).child(field("FOLDER FOR SHARED SESSIONS", this.folder, true))
          .child(div().text_size(12).text_color(red).child(fail.message));
      } else {
        const text = fail.kind === "NoAccess" && f ? `Ask the owner of ${f.repo} to add you on GitHub, then try again.` : fail.message;
        fixArea = box("Can't join yet", text);
      }
    }
    const title = f ? `${f.owner} is pairing on ${f.name}` : "Joining a shared space";
    return v_flex().gap(16)
      .child(v_flex().gap(4)
        .child(div().text_size(17).font_semibold().text_color(t.foreground).child(title))
        .when(Boolean(f), (el) => el.child(div().font_family("JetBrains Mono").text_size(12).text_color(t.muted).child(`${f.repo} · ${f.branch}`))))
      .child(rows)
      .when(Boolean(fixArea), (el) => el.child(fixArea))
      .when(Boolean(engineRow), (el) => el.child(engineRow))
      .child(h_flex().items_center().justify_between().child(cancel).child(action));
  }
}
