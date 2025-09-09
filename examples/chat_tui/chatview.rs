use ratatui::{layout::Size, prelude::*};
use tui_scrollview::{ScrollView, ScrollViewState};

#[derive(Debug, Default)]
pub struct ChatViewWidget {
    pub content: Vec<String>,
}

impl StatefulWidget for ChatViewWidget {
    type State = ScrollViewState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        // let layout = Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]);
        // let [title, body] = layout.areas(area);

        // self.title().render(title, buf);
        let width = if buf.area.height < self.content.len() as u16 {
            buf.area.width - 3
        } else {
            buf.area.width - 2
        };
        let mut scroll_view = ScrollView::new(Size::new(width, self.content.len() as u16));
        let scroll_buf = scroll_view.buf_mut();
        self.content.iter().enumerate().for_each(|(pos, line)| {
            scroll_buf.set_span(0, pos as u16, &Span::raw(line), area.width);
            // buf.scroll_up(1);
        });
        // self.render_widgets_into_scrollview(scroll_view.buf_mut());
        scroll_view.render(area, buf, state)
    }
}
