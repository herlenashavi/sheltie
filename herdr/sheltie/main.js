// Sheltie: pick an AWS target and connect to it in a tab of its own.
// Lists `sheltie ls --json`; type to filter, ↑/↓, Enter connects
// (`sheltie connect`), ctrl+t only opens the tunnel (`sheltie tunnel`),
// ctrl+e edits the targets. Closing the tab closes the session.

import { View, div } from "gpui-kit";
import { v_flex, h_flex, InputState, Input } from "gpui-base";
import { exec, home, run, theme, props, closeOverlay } from "herdr";

const KIND = { shell: "ansi2", rds: "ansi4", tunnel: "ansi5", eks: "ansi3" };

export default class Sheltie extends View {
  init(_props, cx) {
    this.targets = null;
    this.error = null;
    this.filter = "";
    this.at = 0;
    this.input = InputState.new({ placeholder: "Connect to…" });
    this.input.on("change", (_e, cx) => { this.filter = this.input.value(); this.at = 0; cx.notify(); });
    this.input.on("submit", () => this.go(this.rows()[this.at], "connect"));
    this.input.focus();
    cx.timer.after(600, () => this.input.focus());
    this.handle = cx.focus_handle();
    cx.spawn(async (cx) => { await this.load(); cx.notify(); });
  }

  // Herdr's exec uses your login PATH; fall back to cargo's bin folder.
  async sheltie(args) {
    try {
      return await exec("sheltie", args);
    } catch {
      this.bin = `${home()}/.cargo/bin/sheltie`;
      return await exec(this.bin, args);
    }
  }

  async load() {
    try {
      const r = await this.sheltie(["ls", "--json"]);
      if (r.code === 0) this.targets = JSON.parse(r.stdout);
      else this.error = (r.stderr || r.stdout).trim().replace(/^sheltie: /, "");
    } catch {
      this.error = "sheltie isn't installed: cargo install --path . in the sheltie repo";
    }
  }

  rows() {
    const f = this.filter.toLowerCase();
    return (this.targets ?? []).filter((t) =>
      !f || [t.name, t.kind, t.description ?? "", t.profile ?? ""].join(" ").toLowerCase().includes(f));
  }

  close() {
    closeOverlay(props().overlay);
  }

  go(t, verb) {
    if (verb === "edit") {
      this.close();
      run("tab.new", { name: "sheltie config", argv: [this.bin ?? "sheltie", "edit"] });
      return;
    }
    if (!t) return;
    this.close();
    const name = verb === "tunnel" ? `⇄ ${t.name}` : `${t.kind}:${t.name}`;
    run("tab.new", { name, argv: [this.bin ?? "sheltie", verb, t.name] });
  }

  render() {
    const t = theme();
    const rows = this.rows();
    if (this.at >= rows.length) this.at = Math.max(0, rows.length - 1);

    const row = (x, i) => {
      const on = i === this.at;
      const color = t[KIND[x.kind]] ?? t.muted;
      return h_flex()
        .id(`sh-${x.name}`)
        .px(10).h(34).gap(10).items_center().rounded(7).cursor_pointer()
        .when(on, (el) => el.bg(t.hover))
        .hover((s) => s.bg(t.hover))
        .on_click(() => this.go(x, "connect"))
        .child(div().w(46).flex_none().text_size(10.5).font_semibold().text_color(color).child(x.kind.toUpperCase()))
        .child(div().flex_none().text_color(t.foreground).when(on, (d) => d.font_semibold()).child(x.name))
        .child(div().flex_1().truncate().text_size(11.5).text_color(t.dim).child(x.description ?? ""))
        .child(div().flex_none().text_size(11).text_color(t.dim).child(x.profile ?? ""));
    };

    const body = this.error
      ? div().p(10).text_color(t.blocked).child(this.error)
      : this.targets === null
        ? div().p(10).text_color(t.dim).child("Loading targets…")
        : rows.length === 0
          ? div().p(10).text_color(t.dim).child(this.targets.length ? "No match" : "No targets yet: ctrl+e to add some")
          : v_flex().id("sh-list").gap(2).children(rows.map(row));

    return v_flex()
      .id("sheltie")
      .track_focus(this.handle)
      .w_full().min_h(0).p(18).gap(10)
      .bg(t.popover).text_size(13)
      .on_key_down((e, cx) => {
        const k = e.key;
        const n = Math.max(1, rows.length);
        if (k === "escape") return this.close();
        if (k === "down") this.at = (this.at + 1) % n;
        else if (k === "up") this.at = (this.at + n - 1) % n;
        else if (k === "t" && e.modifiers.control) this.go(rows[this.at], "tunnel");
        else if (k === "e" && e.modifiers.control) this.go(null, "edit");
        else return;
        cx.stop_propagation();
        cx.notify();
      })
      .child(h_flex().items_center()
        .child(div().flex_1().text_size(15).font_semibold().text_color(t.foreground).child("Sheltie"))
        .child(div().text_size(11).text_color(t.dim).child("↵ connect · ⌃T tunnel · ⌃E edit · Esc")))
      .child(h_flex().h(36).flex_none().px(10).items_center().rounded(9).border_1().border_color(t.accent).bg(t.background)
        .child(Input.new(this.input).flex_1().h_full().text_size(12.8).text_color(t.foreground)))
      .child(div().id("sh-scroll").h(Math.min(360, Math.max(1, rows.length) * 36 + 8)).flex_none().overflow_y_scrollbar().child(body));
  }
}
