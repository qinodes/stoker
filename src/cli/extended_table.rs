//! Plain-text table rendering for extended CLI queries.

use crate::output::escape_control_characters;

pub(super) fn print_aligned_table<const N: usize>(headers: [&str; N], rows: Vec<[String; N]>) {
    let rows = rows
        .into_iter()
        .map(|row| row.map(|cell| escape_control_characters(&cell)))
        .collect::<Vec<_>>();
    let mut widths = std::array::from_fn(|index| headers[index].chars().count());
    for row in &rows {
        for (index, cell) in row.iter().enumerate() {
            widths[index] = widths[index].max(cell.chars().count());
        }
    }

    let header = headers.map(str::to_owned);
    let separator = widths.map(|width| "-".repeat(width));
    print_table_row(&header, &widths);
    print_table_row(&separator, &widths);
    for row in &rows {
        print_table_row(row, &widths);
    }
}

fn print_table_row<const N: usize>(cells: &[String; N], widths: &[usize; N]) {
    let mut line = String::new();
    for (index, cell) in cells.iter().enumerate() {
        line.push_str(cell);
        if index + 1 < N {
            line.push_str(&" ".repeat(widths[index] - cell.chars().count() + 2));
        }
    }
    println!("{line}");
}
