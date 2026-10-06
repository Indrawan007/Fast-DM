"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");

const ROOT = path.resolve(__dirname, "..");
const read = (file) => fs.readFileSync(path.join(ROOT, file), "utf8");

test("GTK layout wraps content in narrow Hyprland tiles", () => {
  const flow = read("src/gui/mod.rs");
  const window = read("src/gui/window.rs");
  const row = read("src/gui/download_row.rs");
  const quality = read("src/gui/youtube_dialog.rs");

  assert.match(flow, /gtk4::FlowBox/);
  assert.match(flow, /set_selection_mode\(gtk4::SelectionMode::None\)/);
  assert.match(flow, /set_min_children_per_line\(1\)/);
  assert.match(flow, /set_max_children_per_line\(max_children_per_line\.max\(1\)\)/);
  assert.match(window, /adaptive_flow_box\(6, 8, 8\)/);
  assert.match(window, /adaptive_flow_box\(4, 24, 4\)/);
  assert.match(row, /adaptive_flow_box\(3, 12, 2\)/);
  assert.match(row, /adaptive_flow_box\(6, 6, 6\)/);
  assert.match(window, /form_scroll\.set_max_content_height\(460\)/);
  assert.match(window, /form_scroll\.set_propagate_natural_height\(true\)/);
  assert.match(window, /fn wrapped_check_button[\s\S]*?text\.set_wrap\(true\)/);
  assert.match(quality, /label\.set_wrap\(true\)/);
});
