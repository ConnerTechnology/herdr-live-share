// The Live Share panel for the shown space. Paper: page "Live Share panel"
// (Panel 1 to 4). Two tabs:
//   Share    the space's name and Live Share settings, laid out like the
//            Space settings dialog on "Session start & join"
//   Updates  herdr live share's version and one button that updates the mods
//            and the engine on every machine
// It follows the shown space: switch spaces and it shows that one.
//
// Until herdr-app lets a mod add a button next to Files and Changes, the panel
// is a kept window opened from the palette ("Live Share").
//
// The same mod draws the Who can join menu when opened as a popup with
// props { picker }. Both read the session from shared("share.spaces")[spaceId],
// which every engine reply updates, so a change in one shows in the other.
import { View, div } from "gpui-kit";
import { v_flex, h_flex, InputState, Input } from "gpui-base";
import { props, closeOverlay, openOverlay, theme, checking, run, spaces, panes, machines, shared, setShared } from "herdr";
import { engine, installEngine, checkUpdate, updateLine, updateAll, newer, machineName, releaseUrl, openUrl, VERSION } from "./engine.js";

const LONG = 90_000; // start and change talk to GitHub

/** Where a space is now: its first terminal's live folder, else its own (as the menu mod does). */
function place(s) {
  const tabs = new Set((s.tabs ?? []).map((c) => c.id));
  return panes().find((p) => tabs.has(p.tab) && p.cwd)?.cwd || s.folder;
}

function remember(spaceId, session) {
  setShared("share.spaces", { ...(shared("share.spaces") ?? {}), [spaceId]: session ? { session, at: Date.now() } : null });
}

function recalled(spaceId) {
  return shared("share.spaces")?.[spaceId]?.session ?? null;
}

function whoLabel(who) {
  if (!who || who.anyone) return "Anyone with repo access";
  return who.logins.length === 1 ? "1 person" : `${who.logins.length} people`;
}

/** A steady color per login, from the theme's palette. */
function avatar(t, login) {
  const tones = [t.ansi6, t.ansi3, t.ansi5, t.ansi2, t.ansi4, t.ansi1];
  let h = 0;
  for (const c of login) h = (h * 31 + c.charCodeAt(0)) >>> 0;
  return tones[h % tones.length] ?? t.accent;
}

/** Presses on controls must not drag the panel window. */
const stop = (_e, cx) => cx.stop_propagation();

export default class SharePanel extends View {
  init(p, cx) {
    this.handle = cx.focus_handle();
    if ((p ?? props())?.picker) return this.initPicker(p ?? props(), cx);

    this.tab = (p ?? props())?.tab === "updates" ? "updates" : "share";
    this.space = null;
    this.busy = null; // "start" | "stop" | "change" | "install" while the engine works
    this.problem = null; // a message under the toggle (branch not on GitHub, …)
    this.engineNote = null; // the engine is missing, old or unreachable on this machine
    this.fix = null;
    // Updates tab.
    this.update = null;
    this.line = "Checking…";
    this.updating = null; // "check" | "update"
    this.engines = new Map(); // machine key → { on, text, fix }
    this.switchTo(spaces().find((s) => s.active) ?? null, cx);
    if (checking()) return;
    cx.spawn(async (cx) => {
      await Promise.all([this.checkForUpdate(cx, false), this.checkEngines(cx)]);
    });
  }

  /** Show another space: its name, its session, its machine's engine. */
  switchTo(space, cx) {
    this.space = space;
    this.on = space?.machine ?? null;
    this.folder = space ? place(space) : null;
    this.problem = null;
    this.engineNote = null;
    this.fix = null;
    this.name = recalled(space?.id)?.name ?? space?.name ?? "";
    // One field for the panel's life (InputState.new can't run while drawing); a switch only changes its text.
    if (!this.input) {
      this.input = InputState.new({ value: this.name });
      this.input.on("change", (_e, cx) => { this.name = this.input.value(); cx.notify(); });
      this.input.on("submit", (_e, cx) => this.commitName(cx));
      this.input.on("blur", (_e, cx) => this.commitName(cx));
    } else {
      this.input.set_value(this.name);
    }
    if (checking() || !space) return;
    cx.spawn(async (cx) => this.load(cx));
  }

  get session() {
    return recalled(this.space?.id);
  }

  /** Read the session (local on the space's machine, no network). */
  async load(cx) {
    const id = this.space?.id;
    const r = await engine(["status", "--cwd", this.folder], { on: this.on });
    if (this.space?.id !== id) return; // switched away meanwhile
    if (r.ok) {
      remember(id, r.value.session);
      this.engineNote = null;
      this.fix = null;
      const v = await engine(["version"], { on: this.on });
      if (v.ok && newer(VERSION, v.value.version)) {
        this.fix = "update";
        this.engineNote = `herdr-session ${v.value.version} on ${machineName(this.on)} is older than these mods (${VERSION}).`;
      }
    } else {
      this.fix = r.error.kind === "Missing" ? "install" : null;
      this.engineNote = r.error.message;
    }
    cx.notify();
  }

  /** One engine call that returns a session summary; keeps it, or shows why not. */
  async call(cx, busy, args) {
    const id = this.space.id;
    this.busy = busy;
    this.problem = null;
    cx.notify();
    const r = await engine([...args, "--cwd", this.folder], { on: this.on, timeout: LONG });
    this.busy = null;
    if (r.ok) remember(id, r.value);
    else if (this.space?.id === id) this.problem = r.error.message;
    cx.notify();
    return r.ok;
  }

  toggle(cx) {
    const s = this.session;
    if (this.busy || (s && !s.is_owner)) return;
    cx.spawn(async (cx) => {
      if (s?.open) await this.call(cx, "stop", ["change", "--sharing", "off"]);
      else await this.call(cx, "start", ["start", "--name", this.name.trim() || this.space.name]);
    });
  }

  set(cx, flag, value) {
    if (this.busy) return;
    cx.spawn(async (cx) => this.call(cx, "change", ["change", flag, value]));
  }

  /** The name field renames the space here, and the session when this person owns it. */
  commitName(cx) {
    const name = this.name.trim();
    if (!name || !this.space) return;
    if (name !== this.space.name) run("space.rename", { id: this.space.id, name });
    const s = this.session;
    if (s?.is_owner && s.name !== name) this.set(cx, "--name", name);
  }

  openPicker() {
    const s = this.session;
    if (!s?.is_owner || !s.open) return;
    openOverlay({
      mod: "share-panel",
      kind: "popup",
      width: 280,
      height: 360,
      props: { picker: { spaceId: this.space.id, folder: this.folder, on: this.on } },
    });
  }

  installHere(cx) {
    this.busy = "install";
    this.engineNote = `Installing herdr-session on ${machineName(this.on)}…`;
    cx.notify();
    cx.spawn(async (cx) => {
      const r = await installEngine(this.on);
      this.busy = null;
      if (r.ok) await this.load(cx);
      else { this.engineNote = r.error.message; cx.notify(); }
    });
  }

  // ---------------------------------------------------------------- updates

  async checkForUpdate(cx, force) {
    this.updating = "check";
    this.line = "Checking…";
    cx.notify();
    const r = await checkUpdate(cx, { force });
    this.updating = null;
    this.update = r.update;
    this.line = updateLine(r);
    cx.notify();
  }

  /** Engines are checked only to pick the button's label; they are not listed. */
  async checkEngines(cx) {
    const names = [null, ...machines().filter((m) => m.layout !== "unavailable").map((m) => m.name).filter((n) => n !== null)];
    await Promise.all(names.map(async (on) => {
      const r = await engine(["version"], { on });
      const fix = !r.ok ? (r.error.kind === "Missing" ? "install" : null) : newer(VERSION, r.value.version) ? "update" : null;
      this.engines.set(on ?? "", { on, fix, text: r.ok ? `herdr-session ${r.value.version}` : r.error.message });
    }));
    cx.notify();
  }

  updateEverything(cx) {
    this.updating = "update";
    this.line = `Updating to ${this.update}: the mods here and the engine on every machine…`;
    cx.notify();
    cx.spawn(async (cx) => {
      const r = await updateAll();
      // On success the mods are replaced and this panel reloads as the new version.
      this.updating = null;
      this.line = r.ok ? `Updated to ${this.update}.` : r.error.message;
      cx.notify();
      await this.checkEngines(cx);
    });
  }

  /** The mods are current; bring every engine that is missing or behind up to them. */
  updateEngines(cx) {
    const behind = [...this.engines.values()].filter((e) => e.fix);
    this.updating = "update";
    this.line = "Updating the engine on every machine…";
    cx.notify();
    cx.spawn(async (cx) => {
      await Promise.all(behind.map((e) => installEngine(e.on)));
      await this.checkEngines(cx);
      this.updating = null;
      const left = [...this.engines.values()].filter((e) => e.fix);
      this.line = left.length ? left.map((e) => `${machineName(e.on)}: ${e.text}`).join(" ") : "Up to date.";
      cx.notify();
      if (this.space) await this.load(cx);
    });
  }

  statusLine() {
    const behind = [...this.engines.values()].filter((e) => e.fix);
    if (this.update || this.updating || !behind.length) return this.line;
    const where = behind.map((e) => machineName(e.on)).join(", ");
    return `Up to date here. The engine on ${where} ${behind.some((e) => e.fix === "install") ? "needs installing" : "is behind"}.`;
  }

  close() {
    closeOverlay(props()?.overlay);
  }

  // ---------------------------------------------------------------- drawing

  render(cx) {
    if (props()?.picker) return this.renderPicker(cx);
    const shown = spaces().find((s) => s.active) ?? null;
    if ((shown?.id ?? null) !== (this.space?.id ?? null)) this.switchTo(shown, cx);

    const t = theme();
    const s = this.session;
    const live = Boolean(s?.open);
    const tabButton = (id, label, dot) => {
      const on = this.tab === id;
      return h_flex().id(`tab-${id}`).gap(6).px(10).py(5).items_center().rounded(6)
        .text_size(13).font_medium().bg(on ? t.selected ?? t.border : t.popover).text_color(on ? t.foreground : t.muted)
        .cursor_pointer().hover((st) => st.text_color(t.foreground)).on_mouse_down("left", stop)
        .on_click((_e, cx) => { this.tab = id; cx.notify(); })
        .child(label)
        .when(dot, (el) => el.child(div().w(6).h(6).rounded(3).bg(t.accent)));
    };
    const green = t.ansi2 ?? t.done;
    const header = h_flex().gap(4).px(10).py(8).items_center().border_b_1().border_color(t.border)
      .child(tabButton("share", "Share", false))
      .child(tabButton("updates", "Updates", Boolean(this.update) || [...this.engines.values()].some((e) => e.fix)))
      .child(div().flex_1())
      .child(h_flex().gap(6).items_center().pr(4).text_size(12).font_medium().text_color(live ? green : t.muted)
        .child(div().w(7).h(7).rounded(4).bg(live ? green : t.muted))
        .child(live ? "Live" : "Not shared"))
      .child(div().id("close-panel").pl(8).text_size(14).text_color(t.dim).cursor_pointer()
        .hover((st) => st.text_color(t.foreground)).on_mouse_down("left", stop).on_click(() => this.close()).child("×"));

    return v_flex()
      .id("share-panel")
      .track_focus(this.handle)
      .on_key_down((e, cx) => {
        if (e.key !== "escape") return;
        this.close();
        cx.stop_propagation();
      })
      .size_full()
      .rounded(12)
      .bg(t.popover)
      .child(header)
      .child(v_flex().id("panel-body").flex_1().min_h(0).overflow_y_scroll()
        .child(this.tab === "share" ? this.renderShare(cx, t) : this.renderUpdates(cx, t)));
  }

  renderShare(cx, t) {
    if (!this.space) return div().p(20).text_size(13).text_color(t.muted).child("Open a space to share it.");
    const s = this.session;
    const owner = !s || s.is_owner;
    const on = Boolean(s?.open);
    const sw = (id, value, enabled, onClick) =>
      h_flex().id(id).w(36).h(20).flex_shrink_0().px(2).items_center().rounded(10)
        .when(value, (el) => el.justify_end()).bg(value ? t.accent : t.border)
        .when(enabled, (el) => el.cursor_pointer().on_mouse_down("left", stop).on_click(onClick))
        .child(div().w(16).h(16).rounded(8).bg(value ? t.foreground : t.muted));
    const row = (id, title, detail, control, extra) =>
      h_flex().id(id).gap(16).items_center()
        .child(v_flex().flex_1().min_w(0).gap(2)
          .child(div().text_size(13).font_semibold().text_color(t.foreground).child(title))
          .child(div().text_size(12).text_color(t.muted).child(detail))
          .when(Boolean(extra), (el) => el.child(extra)))
        .child(control);
    const label = (text) => div().text_size(11).font_semibold().text_color(t.dim).child(text);

    const toggleDetail = this.busy === "start" ? "Starting the session…" : this.busy === "stop" ? "Closing the session…"
      : s && !owner ? `Shared by ${s.owner}` : "Let people join this space";
    const problem = this.problem
      ? h_flex().gap(8).items_center().pt(8)
        .child(div().w(6).h(6).flex_shrink_0().rounded(3).bg(t.ansi3 ?? t.working))
        .child(div().min_w(0).text_size(12).font_medium().text_color(t.ansi3 ?? t.working).child(this.problem))
      : s && !owner
        ? div().pt(6).text_size(12).text_color(t.dim).child(`Only ${s.owner} can change the session's settings. Keep in sync is yours.`)
        : null;

    // Always shown; enabled only while sharing is on. Before the first share,
    // the defaults a new session starts with. Owner-only rows stay dim for others.
    const d = s ?? { keep_in_sync: true, auto_close: false, who_can_join: { anyone: true, logins: [] }, link: null };
    const live = on && !this.busy;
    const dim = (el, off) => (off ? el.opacity(0.45) : el);
    const settings = v_flex().gap(18).when(!on, (el) => el.opacity(0.45))
      .child(row("sync", "Keep in sync", "Pull and rebase before each push. Push after each turn.",
        sw("sync-sw", d.keep_in_sync, live, (_e, cx) => this.set(cx, "--keep-in-sync", d.keep_in_sync ? "off" : "on"))))
      .child(dim(row("close", "Close automatically", "Close the session after 14 days with no activity.",
        sw("close-sw", d.auto_close, live && owner, (_e, cx) => this.set(cx, "--auto-close", d.auto_close ? "off" : "on"))), on && !owner))
      .child(dim(v_flex().id("who").gap(8)
        .child(v_flex().gap(2)
          .child(div().text_size(13).font_semibold().text_color(t.foreground).child("Who can join"))
          .child(div().text_size(12).text_color(t.muted).child("Leave empty to let everyone with repo access join.")))
        .child(h_flex().id("who-button").px(12).py(8).items_center().rounded(8).border_1()
          .border_color(t.border).bg(t.background)
          .when(live && owner, (el) => el.cursor_pointer().on_mouse_down("left", stop).on_click(() => this.openPicker()))
          .child(div().flex_1().text_size(13).text_color(t.foreground).child(whoLabel(d.who_can_join)))
          .child(div().text_size(13).text_color(t.muted).child("▾"))), on && !owner))
      .child(h_flex().id("link").pl(12).pr(6).py(6).gap(8).items_center().rounded(8).border_1().border_color(t.border).bg(t.background)
        .child(div().flex_1().min_w(0).font_family("JetBrains Mono").text_size(11)
          .text_color(d.link ? t.foreground : t.muted)
          .child(d.link ? d.link.replace(/^https:\/\//, "") : "herdr.dev/join/…"))
        .child(h_flex().id("copy").flex_shrink_0().px(10).py(5).rounded(6).bg(t.accent).text_size(12).font_semibold().text_color(t.accent_foreground)
          .when(on && Boolean(d.link), (el) => el.cursor_pointer().hover((st) => st.opacity(0.88)).on_mouse_down("left", stop).on_click((_e, cx) => {
            cx.write_to_clipboard(d.link);
            this.copied = true;
            cx.notify();
            cx.timer.after(1500, (cx) => { this.copied = false; cx.notify(); });
          }))
          .child(this.copied ? "Copied" : "Copy")));

    return v_flex().p(20).gap(20)
      .child(v_flex().gap(6)
        .child(label("SPACE NAME"))
        .child(Input.new(this.input).h(40).px(12).rounded(8).border_1().border_color(t.border).bg(t.background).text_size(14).text_color(t.foreground)))
      .child(row("share", "Share this space", toggleDetail,
        dim(sw("share-sw", on, owner && !this.busy && !this.engineNote, (_e, cx) => this.toggle(cx)), Boolean(s) && !owner), problem))
      .child(settings)
      .when(Boolean(this.engineNote), (el) => el.child(v_flex().gap(8)
        .child(div().text_size(12).text_color(t.muted).child(this.engineNote))
        .when(Boolean(this.fix) && this.busy !== "install", (el) => el.child(
          h_flex().id("fix").self_start().h(30).px(12).items_center().rounded(8).border_1().border_color(t.border)
            .text_size(12).text_color(t.foreground).cursor_pointer().on_mouse_down("left", stop).on_click((_e, cx) => this.installHere(cx))
            .child(`${this.fix === "install" ? "Install" : "Update"} on ${machineName(this.on)}`)))));
  }

  renderUpdates(cx, t) {
    const idle = this.updating === null;
    const button = (id, label, onClick, primary = false) =>
      h_flex().id(id).h(32).px(12).items_center().rounded(8).border_1()
        .border_color(primary ? t.accent : t.border).bg(primary ? t.accent : t.popover)
        .text_size(13).font_medium().text_color(primary ? t.accent_foreground : t.foreground)
        .cursor_pointer().hover((s) => s.opacity(0.88)).on_mouse_down("left", stop).on_click(onClick).child(label);
    // One button does everything: a new release updates the mods and every engine;
    // with the mods current, it brings any engine that is missing or behind up to them.
    const enginesBehind = [...this.engines.values()].some((e) => e.fix);
    const actions = this.update
      ? [button("update", `Update to ${this.update}`, (_e, cx) => this.updateEverything(cx), true), button("whats-new", "What's new", () => openUrl(releaseUrl(this.update)))]
      : enginesBehind
        ? [button("update-engines", "Update engines", (_e, cx) => this.updateEngines(cx), true), button("whats-new", "What's new", () => openUrl(releaseUrl(VERSION)))]
        : [button("check", "Check now", (_e, cx) => cx.spawn(async (cx) => this.checkForUpdate(cx, true))), button("whats-new", "What's new", () => openUrl(releaseUrl(VERSION)))];
    return v_flex().p(20).gap(12)
      .child(div().text_size(11).font_semibold().text_color(t.dim).child("HERDR LIVE SHARE"))
      .child(v_flex().gap(2)
        .child(div().text_size(13).font_semibold().text_color(t.foreground).child(`herdr live share ${VERSION}`))
        .child(div().text_size(12).text_color(t.muted).child(this.statusLine())))
      .when(idle, (el) => el.child(h_flex().gap(8).children(actions)));
  }

  // ---------------------------------------------------------------- the Who can join menu

  initPicker(p, cx) {
    this.picker = p.picker;
    this.people = null; // loading
    this.query = "";
    this.search = InputState.new({ placeholder: "Search people" });
    this.search.on("change", (_e, cx) => { this.query = this.search.value(); cx.notify(); });
    this.search.focus();
    if (checking()) return;
    cx.spawn(async (cx) => {
      const r = await engine(["collaborators", "--cwd", this.picker.folder], { on: this.picker.on, timeout: 30_000 });
      this.people = r.ok ? r.value : [];
      this.peopleError = r.ok ? null : r.error.message;
      cx.notify();
    });
  }

  pick(cx, login) {
    const s = recalled(this.picker.spaceId);
    if (!s || this.saving) return;
    const chosen = new Set(s.who_can_join.anyone ? [] : s.who_can_join.logins);
    if (chosen.has(login)) chosen.delete(login);
    else chosen.add(login);
    // Show it at once; the engine's reply confirms it.
    remember(this.picker.spaceId, { ...s, who_can_join: { anyone: chosen.size === 0, logins: [...chosen].sort() } });
    this.saving = true;
    cx.spawn(async (cx) => {
      const r = await engine(["change", "--cwd", this.picker.folder, "--who", chosen.size ? [...chosen].join(",") : "anyone"], { on: this.picker.on, timeout: LONG });
      this.saving = false;
      remember(this.picker.spaceId, r.ok ? r.value : s);
      this.peopleError = r.ok ? null : r.error.message;
      cx.notify();
    });
  }

  renderPicker(cx) {
    const t = theme();
    const s = recalled(this.picker.spaceId);
    const chosen = new Set(s && !s.who_can_join.anyone ? s.who_can_join.logins : []);
    const q = this.query.trim().toLowerCase();
    const people = (this.people ?? []).filter((p) => !q || p.login.toLowerCase().includes(q));
    return v_flex()
      .id("who-picker")
      .track_focus(this.handle)
      .on_key_down((e, cx) => {
        if (e.key !== "escape") return;
        closeOverlay(props()?.overlay);
        cx.stop_propagation();
      })
      .size_full()
      .rounded(8)
      .border_1()
      .border_color(t.accent)
      .bg(t.popover)
      .child(div().p(10).border_b_1().border_color(t.border)
        .child(Input.new(this.search).h(32).px(10).rounded(6).border_1().border_color(t.border).bg(t.background).text_size(13).text_color(t.foreground)))
      .child(v_flex().id("people").flex_1().min_h(0).overflow_y_scroll().py(4)
        .when(this.people === null, (el) => el.child(div().px(12).py(10).text_size(12).text_color(t.muted).child("Loading people with access…")))
        .when(Boolean(this.peopleError), (el) => el.child(div().px(12).py(8).text_size(12).text_color(t.blocked ?? t.ansi1).child(this.peopleError)))
        .children(people.map((p) => {
          const on = chosen.has(p.login);
          return h_flex().id(`person-${p.login}`).h(38).px(10).gap(10).items_center().cursor_pointer()
            .hover((st) => st.bg(t.hover)).on_click((_e, cx) => this.pick(cx, p.login))
            .child(h_flex().w(14).h(14).flex_shrink_0().items_center().justify_center().rounded(3).border_1()
              .border_color(on ? t.accent : t.muted).bg(on ? t.accent : t.popover)
              .when(on, (el) => el.child(div().text_size(10).font_semibold().text_color(t.accent_foreground).child("✓"))))
            .child(div().w(20).h(20).flex_shrink_0().rounded(10).bg(avatar(t, p.login)))
            .child(div().flex_1().min_w(0).font_family("JetBrains Mono").text_size(13).text_color(t.foreground).child(p.login))
            .child(div().text_size(12).text_color(t.muted).child(p.permission === "write" ? "Read/Write" : "Read"));
        })));
  }
}
