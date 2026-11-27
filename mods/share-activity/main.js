// Activity: the shown space's shared session as a timeline, newest first.
// Paper: page "4 · Agents talk & Activity", artboards "Agents talk 1 · Activity shows the
// conversation" and "Activity · Empty, error and offline states".
//
// The engine's `timeline` does the work (engine/src/activity.rs); this window
// only lays it out. It reads local state every 5 s and fetches from GitHub
// every 30 s, and only while the window is visible.
import { View, div, svg } from "gpui-kit";
import { v_flex, h_flex } from "gpui-base";
import { props, closeOverlay, theme, checking, run, spaces, panes, window } from "herdr";
import { engine, installEngine, checkUpdate, newer, shownSpace, machineName, openUrl, clock, VERSION } from "./engine.js";

const LOCAL_EVERY = 5_000;
const SYNC_EVERY = 30_000;
const ICON = (name) => `mods/workspace-chrome/${name}.svg`;

/** Where a space is now: its first terminal's live folder, else its own (as the menu mod does). */
function place(s) {
  const tabs = new Set((s.tabs ?? []).map((c) => c.id));
  return panes().find((p) => tabs.has(p.tab) && p.cwd)?.cwd || s.folder;
}

/** "TODAY", "YESTERDAY", "MONDAY", or "NOV 3" for older days. */
function dayLabel(ms, now) {
  const d = new Date(ms), n = new Date(now);
  const start = (x) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
  const days = Math.round((start(n) - start(d)) / 86_400_000);
  if (days <= 0) return "TODAY";
  if (days === 1) return "YESTERDAY";
  if (days < 7) return ["SUNDAY", "MONDAY", "TUESDAY", "WEDNESDAY", "THURSDAY", "FRIDAY", "SATURDAY"][d.getDay()];
  return `${["JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC"][d.getMonth()]} ${d.getDate()}`;
}

function ago(ms) {
  const m = Math.round(ms / 60_000);
  if (m < 1) return "just now";
  if (m < 60) return `${m} min ago`;
  const h = Math.round(m / 60);
  return h < 24 ? `${h} h ago` : `${Math.round(h / 24)} d ago`;
}

export default class Activity extends View {
  init(_props, cx) {
    this.handle = cx.focus_handle();
    this.handle.focus();
    this.space = null;
    this.timeline = null; // the engine's reply, or null while loading
    this.state = "loading"; // loading | ready | not-shared | no-space | engine
    this.note = null; // why there is no timeline (engine missing, …)
    this.fix = null; // "install" | "update"
    this.net = null;
    this.lastSync = 0;
    this.loading = false;
    this.update = null;
    if (checking()) return;
    cx.spawn(async (cx) => {
      await this.load(cx, true);
      const r = await checkUpdate(cx);
      this.update = r.update;
      cx.notify();
    });
    cx.timer.every(LOCAL_EVERY, (cx) => {
      if (window()?.visible === false || this.loading) return;
      cx.spawn(async (cx) => this.load(cx, Date.now() - this.lastSync >= SYNC_EVERY));
    });
  }

  /** Read the shown space's timeline; `sync` fetches from GitHub first. */
  async load(cx, sync) {
    const s = shownSpace();
    if (s?.id !== this.space?.id) {
      this.space = s;
      this.timeline = null;
      this.state = "loading";
      sync = true;
    }
    if (!s) {
      this.state = "no-space";
      cx.notify();
      return;
    }
    this.loading = true;
    const args = ["timeline", "--cwd", place(s), ...(sync ? ["--sync"] : [])];
    const r = await engine(args, { on: s.machine ?? null, timeout: sync ? 30_000 : 10_000 });
    this.loading = false;
    if (s.id !== shownSpace()?.id) return; // switched while loading: the next tick reads the new one
    if (sync) this.lastSync = Date.now();
    if (r.ok) {
      this.timeline = r.value.timeline;
      this.net = r.value.net;
      this.state = "ready";
      this.fix = null;
      this.note = null;
    } else if (r.error.kind === "NotShared" || r.error.kind === "NotARepo" || r.error.kind === "NoBranch") {
      this.state = "not-shared";
    } else {
      this.state = "engine";
      this.note = r.error.message;
      this.fix = r.error.kind === "Missing" ? "install" : null;
      if (r.error.kind === "Version") this.fix = "update";
    }
    cx.notify();
  }

  installHere(cx) {
    this.note = `Installing herdr-session on ${machineName(this.space?.machine ?? null)}…`;
    this.fix = null;
    cx.notify();
    cx.spawn(async (cx) => {
      const r = await installEngine(this.space?.machine ?? null);
      if (!r.ok) this.note = r.error.message;
      await this.load(cx, true);
    });
  }

  close() {
    closeOverlay(props()?.overlay);
  }

  // ---------------------------------------------------------------- drawing

  render(cx) {
    const t = theme();
    const tl = this.timeline;
    const header = v_flex().gap(2).px(16).pt(14).pb(12).border_b_1().border_color(t.border)
      .child(div().text_size(15).font_semibold().text_color(t.foreground).child("Activity"))
      .child(div().text_size(12).text_color(t.dim).whitespace_nowrap().overflow_hidden().text_ellipsis()
        .child(tl ? `${tl.name} · ${tl.branch}${tl.open ? "" : " · closed"}` : this.space?.name ?? ""));

    return v_flex()
      .id("share-activity")
      .track_focus(this.handle)
      .on_key_down((e, cx) => {
        if (e.key !== "escape") return;
        this.close();
        cx.stop_propagation();
      })
      .size_full()
      .bg(t.popover)
      .child(header)
      .child(this.state === "ready" ? this.renderFeed(cx, t, tl) : this.renderEmpty(cx, t))
      .child(this.renderFooter(t));
  }

  renderEmpty(cx, t) {
    const message = (title, detail) => v_flex().flex_1().gap(6).p(16)
      .child(div().text_size(13).font_semibold().text_color(t.foreground).child(title))
      .child(div().text_size(12).text_color(t.muted).child(detail));
    switch (this.state) {
      case "no-space":
        return message("No space shown", "Open a space to see its shared session.");
      case "not-shared":
        return message("This space isn't shared", "Turn on sharing in Live Share: Share this space, and the session shows here.")
          .child(div().id("share").mt(6).text_size(12).font_semibold().underline().text_color(t.accent).cursor_pointer()
            .on_click(() => run("mod.open", { name: "share-settings" })).child("Share this space"));
      case "engine":
        return message("Can't read the session", this.note ?? "")
          .when(Boolean(this.fix), (el) => el.child(
            h_flex().id("fix").mt(6).h(30).px(12).items_center().self_start().rounded(8).border_1().border_color(t.border)
              .text_size(12).text_color(t.foreground).cursor_pointer().on_click((_e, cx) => this.installHere(cx))
              .child(`${this.fix === "install" ? "Install" : "Update"} on ${machineName(this.space?.machine ?? null)}`)));
      default:
        return message("Loading…", "Reading the shared session.");
    }
  }

  renderFeed(cx, t, tl) {
    const tones = [t.ansi4, t.ansi6, t.ansi3, t.ansi5, t.ansi2, t.ansi1].map((c) => c ?? t.accent);
    const tone = (login) => (login ? tones[Math.max(0, tl.members.indexOf(login)) % tones.length] : t.dim);
    const avatar = (login, size) => h_flex().w(size).h(size).flex_none().items_center().justify_center().rounded(size / 2)
      .bg(tone(login)).text_size(size <= 20 ? 10 : 11).font_semibold().text_color(t.background)
      .child((login ?? "·")[0].toUpperCase());
    const who = (login, agent) => {
      if (!login) return "";
      const mine = login === tl.me;
      return agent ? (mine ? "Your agent" : `${login}'s agent`) : (mine ? "You" : login);
    };
    const labelColor = { "Question": t.accent, "Second opinion": t.ansi3 ?? t.accent, "Finding": t.ansi2 ?? t.accent, "Working on": t.ansi4 ?? t.accent };
    const label = (text) => div().flex_none().px(7).rounded(9).border_1().border_color(labelColor[text]).text_size(11).font_medium()
      .text_color(labelColor[text]).child(text);
    const time = (at) => div().flex_none().text_size(12).text_color(t.dim).child(clock(at));
    const quote = (login, text) => div().border_l_2().border_color(tone(login)).pl(10).text_size(12.5).text_color(t.muted)
      .whitespace_normal().child(text);

    const answer = (a) => h_flex().gap(10).items_start()
      .child(avatar(a.login, 20))
      .child(v_flex().flex_1().min_w(0).gap(2)
        .child(h_flex().gap(8).items_center()
          .child(div().flex_1().min_w(0).text_size(12.5).text_color(t.muted)
            .child(h_flex().gap(4).child(div().font_semibold().text_color(t.foreground).child(who(a.login, a.agent))).child("answered")))
          .when(a.background, (el) => el.child(h_flex().gap(4).items_center().text_size(11).text_color(t.muted)
            .child(svg(ICON("clock")).size(12).text_color(t.muted)).child("in the background")))
          .child(time(a.at)))
        .child(div().text_size(12.5).text_color(t.foreground).child(a.text)));

    const row = (e, i) => h_flex().id(`entry-${i}`).gap(10).items_start().py(8)
      .child(avatar(e.login, 24))
      .child(v_flex().flex_1().min_w(0).gap(6)
        .child(h_flex().gap(8).items_center()
          .child(div().flex_1().min_w(0).text_size(13).text_color(t.muted)
            .child(e.login
              ? h_flex().gap(4).flex_wrap().child(div().font_semibold().text_color(t.foreground).child(who(e.login, e.agent))).child(e.verb)
              : div().child(e.verb)))
          .when(Boolean(e.label), (el) => el.child(label(e.label)))
          .child(time(e.at)))
        .when(Boolean(e.quote), (el) => el.child(quote(e.login, e.quote)))
        .when(Boolean(e.link), (el) => el.child(div().id(`link-${i}`).pl(12).text_size(12).underline().text_color(t.accent).cursor_pointer()
          .on_click(() => openUrl(e.link)).child("View changes")))
        .when(e.answers.length > 0, (el) => el.child(v_flex().gap(10).mt(2).p(10).rounded(8).bg(t.hover)
          .children(e.answers.map(answer)))));

    const now = Date.now();
    const children = [];
    let day = null;
    tl.entries.forEach((e, i) => {
      const d = dayLabel(e.at, now);
      if (d !== day) {
        day = d;
        children.push(div().pt(10).pb(2).text_size(11).font_semibold().text_color(t.dim).child(d));
      }
      children.push(row(e, i));
    });
    if (children.length === 0) children.push(div().py(10).text_size(12).text_color(t.muted).child("Nothing yet."));
    return v_flex().id("feed").flex_1().min_h(0).overflow_y_scroll().px(16).pb(12).children(children);
  }

  renderFooter(t) {
    const offline = this.net?.last_error
      ? `Can't reach GitHub${this.net.last_ok ? `; last fetched ${ago(Date.now() - this.net.last_ok)}` : ""}.`
      : null;
    return h_flex().gap(8).items_center().px(16).py(10).border_t_1().border_color(t.border).text_size(11)
      .child(div().flex_1().min_w(0).whitespace_nowrap().overflow_hidden().text_ellipsis()
        .text_color(offline ? (t.ansi3 ?? t.muted) : t.dim)
        .child(offline ?? `herdr live share ${VERSION}`))
      .when(Boolean(this.update), (el) => el.child(div().id("updates").flex_none().underline().font_semibold().text_color(t.accent)
        .cursor_pointer().on_click(() => { this.close(); run("mod.open", { name: "share-updates" }); }).child("See update")));
  }
}
