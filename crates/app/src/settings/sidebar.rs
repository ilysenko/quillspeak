use gtk::prelude::*;
use gtk4 as gtk;

#[derive(Clone)]
pub(super) struct SettingsSidebar {
    container: gtk::ScrolledWindow,
    list: gtk::ListBox,
}

pub(super) struct SidebarSection {
    title: String,
    pages: Vec<SidebarPage>,
}

pub(super) struct SidebarPage {
    id: String,
    title: String,
}

impl SettingsSidebar {
    pub(super) fn new(stack: &gtk::Stack) -> Self {
        let list = gtk::ListBox::new();
        list.set_selection_mode(gtk::SelectionMode::Single);
        list.add_css_class("navigation-sidebar");

        list.connect_row_selected({
            let stack = stack.clone();
            move |_, row| {
                let Some(row) = row else {
                    return;
                };
                let page_id = row.widget_name();
                if stack.child_by_name(&page_id).is_some() {
                    stack.set_visible_child_name(&page_id);
                }
            }
        });

        let container = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .width_request(190)
            .vexpand(true)
            .child(&list)
            .build();

        Self { container, list }
    }

    pub(super) fn widget(&self) -> &gtk::ScrolledWindow {
        &self.container
    }

    pub(super) fn set_sections(&self, sections: &[SidebarSection], selected_page: &str) {
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }

        let mut selected_row = None;
        for (section_index, section) in sections.iter().enumerate() {
            self.list.append(&section_header(
                &section.title,
                section_index == 0,
                section.pages.is_empty(),
            ));

            for page in &section.pages {
                let row = page_row(page);
                if page.id == selected_page {
                    selected_row = Some(row.clone());
                }
                self.list.append(&row);
            }
        }

        self.list.select_row(selected_row.as_ref());
    }
}

impl SidebarSection {
    pub(super) fn new(title: impl Into<String>, pages: Vec<SidebarPage>) -> Self {
        Self {
            title: title.into(),
            pages,
        }
    }
}

impl SidebarPage {
    pub(super) fn new(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
        }
    }
}

fn section_header(title: &str, is_first: bool, is_empty: bool) -> gtk::ListBoxRow {
    let label = gtk::Label::builder()
        .label(title)
        .xalign(0.0)
        .margin_top(if is_first { 10 } else { 18 })
        .margin_bottom(if is_empty { 0 } else { 4 })
        .margin_start(12)
        .margin_end(12)
        .build();
    label.add_css_class("caption-heading");
    label.add_css_class("dim-label");

    gtk::ListBoxRow::builder()
        .activatable(false)
        .selectable(false)
        .child(&label)
        .build()
}

fn page_row(page: &SidebarPage) -> gtk::ListBoxRow {
    let label = gtk::Label::builder()
        .label(&page.title)
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .margin_top(7)
        .margin_bottom(7)
        .margin_start(12)
        .margin_end(12)
        .build();

    gtk::ListBoxRow::builder()
        .name(&page.id)
        .activatable(true)
        .selectable(true)
        .child(&label)
        .build()
}
