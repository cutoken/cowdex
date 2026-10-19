//! Repair the glyph beneath cursor-style commands on older JediTerm terminals.

use std::io;
use std::io::Write;

use crossterm::cursor::SetCursorStyle;
use ratatui::backend::Backend;
use ratatui::buffer::CellDiffOption;
use ratatui::buffer::CellWidth;
use ratatui::layout::Position;

use super::DrawCommand;
use super::Terminal;
use super::draw;

impl<B> Terminal<B>
where
    B: Backend<Error = io::Error> + Write,
{
    pub(super) fn set_cursor_style_with_repair(
        &mut self,
        cursor_style: SetCursorStyle,
    ) -> io::Result<()> {
        // Older JediTerm versions print DECSCUSR's space intermediate at the cursor. Apply the
        // style over an owned glyph and redraw it, including on otherwise unchanged frames.
        let buffer = &self.buffers[self.current];
        let anchor = (0..buffer.area.height).find_map(|row| {
            let row_start = usize::from(row) * usize::from(buffer.area.width);
            let mut column = 0;
            while column < usize::from(buffer.area.width) {
                let cell = &buffer.content[row_start + column];
                let width = usize::from(cell.cell_width());
                if cell.diff_option != CellDiffOption::Skip
                    && width > 0
                    && column + width <= usize::from(buffer.area.width)
                {
                    let (x, y) = buffer.pos_of(row_start + column);
                    return Some((Position { x, y }, cell.clone()));
                }
                column += width.max(1);
            }
            None
        });

        if let Some((anchor, cell)) = anchor {
            self.set_cursor_position(anchor)?;
            self.set_cursor_style(cursor_style)?;
            let Position { x, y } = anchor;
            draw(
                &mut self.backend,
                std::iter::once(DrawCommand::Put { x, y, cell }),
            )?;
        }

        Ok(())
    }
}
