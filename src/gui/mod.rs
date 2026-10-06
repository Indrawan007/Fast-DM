pub mod css;
pub mod download_row;
pub mod window;
#[allow(deprecated)] // gtk4::Dialog deprecated sejak 4.10
pub mod youtube_dialog;

/// Baris horizontal yang membungkus anak saat jendela ditile dengan lebar sempit.
pub(crate) fn adaptive_flow_box(
    max_children_per_line: u32,
    column_spacing: u32,
    row_spacing: u32,
) -> gtk4::FlowBox {
    let flow = gtk4::FlowBox::new();
    flow.set_selection_mode(gtk4::SelectionMode::None);
    flow.set_homogeneous(false);
    flow.set_min_children_per_line(1);
    flow.set_max_children_per_line(max_children_per_line.max(1));
    flow.set_column_spacing(column_spacing);
    flow.set_row_spacing(row_spacing);
    flow
}
