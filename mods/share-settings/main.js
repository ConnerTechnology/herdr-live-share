// Space settings: share the shown space, and its session's settings.
// Paper: "Share 0 · Space settings, never shared", "Share 0 · Branch not on
// GitHub", "Space settings · sharing on / off / choosing who can join".
//
// The Who can join menu is a Popover anchored to its dropdown. The session is
// kept in shared("share.spaces")[spaceId], which every engine reply updates.
import { View, div, svg } from "gpui-kit";
import { v_flex, h_flex, InputState, Input, Popover } from "gpui-base";
import { props, closeOverlay, theme, checking, run, spaces, panes, shared, setShared } from "herdr";
import { engine, installEngine, checkUpdate, newer, machineName, VERSION } from "./engine.js";

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

/** herdr's own icons, shipped with its workspace-chrome mod. */
const ICON = (name) => `mods/workspace-chrome/${name}.svg`;

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

export default class ShareSettings extends View {
  init(p, cx) {
    this.handle = cx.focus_handle();

    this.space = spaces().find((s) => s.active) ?? null;
    this.on = this.space?.machine ?? null;
    this.folder = this.space ? place(this.space) : null;
    this.busy = null; // "start" | "stop" | "change" while the engine works
    this.pending = null; // { id, value }: the switch a click just moved, until the engine answers
    this.problem = null; // a message under the toggle (branch not on GitHub, …)
    this.engineNote = null; // the engine is missing, old or unreachable on this machine
    this.fix = null;
    this.update = null;
    this.updateNote = null;
    this.name = recalled(this.space?.id)?.name ?? this.space?.name ?? "";
    this.input = InputState.new({ value: this.name });
    this.input.on("change", (_e, cx) => { this.name = this.input.value(); cx.notify(); });
    this.input.on("submit", (_e, cx) => this.commitName(cx));
    this.input.on("blur", (_e, cx) => this.commitName(cx));
    this.pickerOpen = false;
    this.people = null; // loading
    this.query = "";
    this.search = InputState.new({ placeholder: "Search people" });
    this.search.on("change", (_e, cx) => { this.query = this.search.value(); cx.notify(); });
    this.handle.focus();
    if (checking() || !this.space) return;
    cx.spawn(async (cx) => {
      await this.load(cx);
      const r = await checkUpdate(cx);
      this.update = r.update;
      this.updateNote = r.update ? `Version ${r.update} is available` : "Up to date";
      cx.notify();
    });
  }

  get session() {
    return recalled(this.space?.id);
  }

  /** Read the session (local on the space's machine, no network). */
  async load(cx) {
    const r = await engine(["status", "--cwd", this.folder], { on: this.on });
    if (r.ok) {
      remember(this.space.id, r.value.session);
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
    this.busy = busy;
    this.problem = null;
    cx.notify();
    const r = await engine([...args, "--cwd", this.folder], { on: this.on, timeout: LONG });
    this.busy = null;
    this.pending = null;
    if (r.ok) remember(this.space.id, r.value);
    else this.problem = r.error.message;
    cx.notify();
    return r.ok;
  }

  toggle(cx) {
    const s = this.session;
    if (this.busy || (s && !s.is_owner)) return;
    this.pending = { id: "share-sw", value: !s?.open };
    cx.spawn(async (cx) => {
      if (s?.open) await this.call(cx, "stop", ["change", "--sharing", "off"]);
      else await this.call(cx, "start", ["start", "--name", this.name.trim() || this.space.name]);
    });
  }

  /** One setting change. `sw` names the switch it moves, which shows the new value at once. */
  set(cx, flag, value, sw) {
    if (this.busy) return;
    if (sw) this.pending = { id: sw, value: value === "on" };
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

  setPicker(cx, open) {
    const s = this.session;
    this.pickerOpen = open && Boolean(s?.is_owner && s.open);
    if (this.pickerOpen) {
      this.search.focus();
      if (this.people === null) cx.spawn(async (cx) => {
        const r = await engine(["collaborators", "--cwd", this.folder], { on: this.on, timeout: 30_000 });
        this.people = r.ok ? r.value : [];
        this.peopleError = r.ok ? null : r.error.message;
        cx.notify();
      });
    }
    cx.notify();
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

  openUpdates() {
    this.close();
    run("mod.open", { name: "share-updates" });
  }

  close() {
    closeOverlay(props()?.overlay);
  }

  // ---------------------------------------------------------------- the dialog

  render(cx) {
    const t = theme();
    const s = this.session;
    const owner = !s || s.is_owner;
    const on = Boolean(s?.open);
    const button = (id, label, primary, onClick) =>
      h_flex().id(id).h(34).px(14).items_center().rounded(8).border_1()
        .border_color(primary ? t.accent : t.border).bg(primary ? t.accent : t.popover)
        .text_size(13).font_medium().text_color(primary ? t.accent_foreground : t.foreground)
        .cursor_pointer().hover((st) => st.opacity(0.88)).on_click(onClick).child(label);
    // The knob slides; while its change runs the switch holds the new value,
    // pulses, and every switch ignores clicks (one engine call at a time).
    const sw = (id, value, enabled, onClick) => {
      const waiting = this.pending?.id === id;
      const shown = waiting ? this.pending.value : value;
      return div().id(id).relative().w(36).h(20).flex_shrink_0().rounded(10).bg(shown ? t.accent : t.border)
        .when(enabled && !this.busy, (el) => el.cursor_pointer().on_click(onClick))
        .child(div().id(`${id}-knob`).absolute().top(2).left(shown ? 18 : 2).w(16).h(16).rounded(8)
          .bg(shown ? t.foreground : t.muted)
          .transition("left", { duration: 160, easing: "ease-out" })
          .when(waiting, (el) => el.loop("opacity", { from: 1, to: 0.45, period: 900 })));
    };
    const row = (id, title, detail, control, extra) =>
      h_flex().id(id).gap(16).items_center()
        .child(v_flex().flex_1().min_w(0).gap(2)
          .child(div().text_size(13).font_semibold().text_color(t.foreground).child(title))
          .child(div().text_size(12).text_color(t.muted).child(detail))
          .when(Boolean(extra), (el) => el.child(extra)))
        .child(control);
    const label = (text) => div().text_size(11).font_semibold().text_color(t.dim).child(text);

    const toggleDetail = this.busy === "start" ? "Starting the session…" : this.busy === "stop" ? "Closing the session…" : "Let people join this space";
    const problem = this.problem
      ? h_flex().gap(8).items_center().pt(8)
        .child(div().w(6).h(6).flex_shrink_0().rounded(3).bg(t.ansi3 ?? t.working))
        .child(div().text_size(12).font_medium().text_color(t.ansi3 ?? t.working).child(this.problem))
      : null;

    // Always shown; enabled only while sharing is on. Before the first share,
    // the defaults a new session starts with.
    const d = s ?? { keep_in_sync: true, auto_close: false, who_can_join: { anyone: true, logins: [] }, link: null };
    const live = on && !this.busy;
    const settings = v_flex().gap(18).when(!on, (el) => el.opacity(0.45))
      .child(row("sync", "Keep in sync", "Pull and rebase before each push. Push after each turn.",
        sw("sync-sw", d.keep_in_sync, live, (_e, cx) => this.set(cx, "--keep-in-sync", d.keep_in_sync ? "off" : "on", "sync-sw"))))
      .child(row("close", "Close automatically", "Close the session after 14 days with no activity.",
        sw("close-sw", d.auto_close, live && owner, (_e, cx) => this.set(cx, "--auto-close", d.auto_close ? "off" : "on", "close-sw"))))
      .child(h_flex().id("who").gap(16).items_start()
        .child(v_flex().flex_1().min_w(0).gap(2)
          .child(div().text_size(13).font_semibold().text_color(t.foreground).child("Who can join"))
          .child(div().text_size(12).text_color(t.muted).child("Anyone with repo access, or only people you pick.")))
        .child(Popover.new("who-menu").anchor("top_right")
          .open(this.pickerOpen && live && owner)
          .on_open_change((open, cx) => this.setPicker(cx, open))
          .trigger(h_flex().id("who-button").w(220).h(36).flex_shrink_0().px(12).gap(8).items_center().rounded(8).border_1()
            .border_color(t.border).bg(this.pickerOpen ? t.hover : t.background)
            .when(live && owner, (el) => el.cursor_pointer().hover((st) => st.bg(t.hover)))
            .child(div().flex_1().min_w(0).whitespace_nowrap().overflow_hidden().text_ellipsis().text_size(13).text_color(t.foreground).child(whoLabel(d.who_can_join)))
            .child(svg(ICON("chevron-down")).size(14).flex_none().text_color(t.muted)))
          .content(this.renderPicker(cx))))
      .child(h_flex().id("link").px(12).py(10).gap(12).items_center().rounded(8).border_1().border_color(t.border).bg(t.background)
        .child(div().flex_1().min_w(0).whitespace_nowrap().overflow_hidden().text_ellipsis_middle().font_family("JetBrains Mono").text_size(13)
          .text_color(d.link ? t.foreground : t.muted)
          .child(d.link ? d.link.replace(/^https:\/\//, "") : "herdr.dev/join/…"))
        // Fixed width, so swapping Copy for Copied doesn't move anything.
        .child(h_flex().id("copy").w(92).h(30).flex_shrink_0().gap(6).items_center().justify_center().rounded(6).bg(t.accent)
          .text_size(12).font_semibold().text_color(t.accent_foreground)
          .when(on && Boolean(d.link), (el) => el.cursor_pointer().hover((st) => st.opacity(0.88)).on_click((_e, cx) => {
            cx.write_to_clipboard(d.link);
            const n = (this.copied = (this.copied || 0) + 1);
            cx.notify();
            cx.timer.after(2000, (cx) => { if (this.copied === n) { this.copied = 0; cx.notify(); } });
          }))
          .child(svg(this.copied ? ICON("check") : "mods/share-settings/copy.svg").size(14).flex_none())
          .child(this.copied ? "Copied" : "Copy")));

    return v_flex()
      .id("share-settings")
      .track_focus(this.handle)
      .on_key_down((e, cx) => {
        if (e.key !== "escape") return;
        if (this.pickerOpen) this.setPicker(cx, false);
        else this.close();
        cx.stop_propagation();
      })
      .size_full()
      .p(24)
      .gap(20)
      .rounded(12)
      .bg(t.popover)
      .child(div().self_center().text_size(17).font_semibold().text_color(t.foreground).child("Space settings"))
      .when(!this.space, (el) => el.child(div().text_size(13).text_color(t.muted).child("Open a space first.")))
      .when(Boolean(this.space), (el) => el
        .child(v_flex().gap(6)
          .child(label("SPACE NAME"))
          .child(Input.new(this.input).h(40).px(12).rounded(8).border_1().border_color(t.border).bg(t.background).text_size(14).text_color(t.foreground)))
        .child(row("share", "Share this space", toggleDetail,
          sw("share-sw", on, owner && !this.engineNote, (_e, cx) => this.toggle(cx)), problem))
        .when(Boolean(s) && !owner, (el) => el.child(div().text_size(12).text_color(t.dim).child(`${s.owner} shares this space. Only they can change its settings; Keep in sync is yours.`)))
        .child(settings)
        .when(Boolean(this.engineNote), (el) => el.child(div().text_size(12).text_color(t.dim).child(this.engineNote))))
      .child(
        h_flex().items_center().gap(12).pt(16).border_t_1().border_color(t.border)
          .child(h_flex().flex_1().min_w(0).gap(6).items_center().text_size(12)
            .when(Boolean(this.update), (el) => el.child(div().w(6).h(6).rounded(3).bg(t.accent)))
            .when(Boolean(this.updateNote), (el) => el
              .child(div().min_w(0).whitespace_nowrap().overflow_hidden().text_ellipsis().text_color(t.muted).child(`herdr live share ${VERSION} · ${this.updateNote}`))
              .child(div().id("updates").flex_none().whitespace_nowrap().cursor_pointer().on_click(() => this.openUpdates()).underline().font_semibold()
                .text_color(this.update ? t.accent : t.foreground).hover((st) => st.opacity(0.8))
                .child(this.update ? "See update" : "Check for updates"))))
          .when(Boolean(this.fix) && this.busy !== "install", (el) => el.child(
            button("fix", this.fix === "install" ? `Install on ${machineName(this.on)}` : `Update on ${machineName(this.on)}`, false, (_e, cx) => this.installHere(cx))))
          .child(button("close-dialog", "Close", false, () => this.close())),
      );
  }

  // ---------------------------------------------------------------- the Who can join menu

  pick(cx, login) {
    const s = this.session;
    if (!s || this.saving) return;
    const chosen = new Set(s.who_can_join.anyone ? [] : s.who_can_join.logins);
    if (chosen.has(login)) chosen.delete(login);
    else chosen.add(login);
    // Show it at once; the engine's reply confirms it.
    remember(this.space.id, { ...s, who_can_join: { anyone: chosen.size === 0, logins: [...chosen].sort() } });
    this.saving = true;
    cx.spawn(async (cx) => {
      const r = await engine(["change", "--cwd", this.folder, "--who", chosen.size ? [...chosen].join(",") : "anyone"], { on: this.on, timeout: LONG });
      this.saving = false;
      remember(this.space.id, r.ok ? r.value : s);
      this.peopleError = r.ok ? null : r.error.message;
      cx.notify();
    });
  }

  /** The Who can join menu, drawn like herdr's own menus. */
  renderPicker(cx) {
    const t = theme();
    const s = this.session;
    // No one picked means anyone with repo access, which the button says.
    const chosen = new Set(s && !s.who_can_join.anyone ? s.who_can_join.logins : []);
    const q = this.query.trim().toLowerCase();
    const people = (this.people ?? []).filter((p) => !q || p.login.toLowerCase().includes(q));
    const check = (on) => div().w(14).flex_none().when(on, (el) => el.child(svg(ICON("check")).size(14).text_color(t.accent)));
    const item = (id, onClick) => h_flex().id(id).h(32).px(8).gap(9).rounded(7).items_center().cursor_pointer()
      .hover((st) => st.bg(t.hover)).on_click(onClick);
    const note = (text, color) => div().px(8).py(8).text_size(12).text_color(color).child(text);
    return v_flex()
      .id("who-picker")
      .w(300)
      .max_h(340)
      .mt(6)
      .p(5)
      .rounded(10)
      .border_1()
      .border_color(t.border)
      .bg(t.popover)
      .shadow_lg()
      .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
      .child(h_flex().h(32).px(8).gap(8).items_center()
        .child(svg(ICON("search")).size(14).flex_none().text_color(t.muted))
        .child(Input.new(this.search).flex_1().h(30).border_0().bg(t.popover).text_size(13).text_color(t.foreground)))
      .child(div().mx(3).my(4).h(1).bg(t.border))
      .child(v_flex().id("people").flex_1().min_h(0).overflow_y_scroll()
        .when(this.people === null, (el) => el.child(note("Loading people with access…", t.muted)))
        .when(this.people !== null && !people.length && !this.peopleError, (el) => el.child(note(q ? "No one matches." : "No one else has access.", t.muted)))
        .when(Boolean(this.peopleError), (el) => el.child(note(this.peopleError, t.blocked ?? t.ansi1)))
        .children(people.map((p) => item(`person-${p.login}`, (_e, cx) => this.pick(cx, p.login))
          .child(h_flex().w(20).h(20).flex_none().items_center().justify_center().rounded(10).bg(avatar(t, p.login))
            .text_size(11).font_semibold().text_color(t.background).child(p.login[0].toUpperCase()))
          .child(div().flex_1().min_w(0).whitespace_nowrap().overflow_hidden().text_ellipsis().text_size(13).text_color(t.foreground).child(p.login))
          .child(div().text_size(11).text_color(t.dim).child(p.permission === "write" ? "Read/Write" : "Read"))
          .child(check(chosen.has(p.login))))));
  }
}
