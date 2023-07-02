use crate::csv::{CsvLensReader, Row};
use crate::find;
use crate::input::Control;

use anyhow::Result;
use regex::Regex;
use std::cmp::min;
use std::time::Instant;

struct RowsFilter {
    indices: Vec<u64>,
    total: usize,
}

impl RowsFilter {
    fn new(finder: &find::Finder, rows_from: u64, num_rows: u64) -> RowsFilter {
        let total = finder.count();
        let indices = finder.get_subset_found(rows_from as usize, num_rows as usize);
        RowsFilter { indices, total }
    }
}

#[derive(Debug)]
pub struct ColumnsFilter {
    pattern: Regex,
    indices: Vec<usize>,
    filtered_headers: Vec<String>,
    num_columns_before_filter: usize,
    disabled_because_no_match: bool,
}

impl ColumnsFilter {
    fn new(pattern: Regex, headers: &[String]) -> Self {
        let mut indices = vec![];
        let mut filtered_headers: Vec<String> = vec![];
        for (i, header) in headers.iter().enumerate() {
            if pattern.is_match(header) {
                indices.push(i);
                filtered_headers.push(header.clone());
            }
        }
        let disabled_because_no_match;
        if indices.is_empty() {
            indices = (0..headers.len()).collect();
            filtered_headers = headers.into();
            disabled_because_no_match = true;
        } else {
            disabled_because_no_match = false;
        }
        Self {
            pattern,
            indices,
            filtered_headers,
            num_columns_before_filter: headers.len(),
            disabled_because_no_match,
        }
    }

    fn filtered_headers(&self) -> &Vec<String> {
        &self.filtered_headers
    }

    fn indices(&self) -> &Vec<usize> {
        &self.indices
    }

    pub fn pattern(&self) -> Regex {
        self.pattern.to_owned()
    }

    pub fn num_filtered(&self) -> usize {
        self.indices.len()
    }

    pub fn num_original(&self) -> usize {
        self.num_columns_before_filter
    }

    pub fn disabled_because_no_match(&self) -> bool {
        self.disabled_because_no_match
    }
}

#[derive(Clone)]
pub struct SelectionDimension {
    index: Option<u64>,
    pub bound: u64,
}

impl SelectionDimension {
    /// The currently selected index
    ///
    /// This index is dumb as in it is always between 0 and bound - 1 and
    /// has nothing to do with the actual record number in the data.
    pub fn index(&self) -> Option<u64> {
        self.index
    }

    /// Set selected to the given index and adjust it to be within bounds
    pub fn set_index(&mut self, index: u64) {
        self.index = Some(min(index, self.bound.saturating_sub(1)));
    }

    /// Set the maximum allowed value for for index
    pub fn set_bound(&mut self, bound: u64) {
        self.bound = bound;
        if let Some(i) = self.index {
            self.set_index(i);
        }
    }

    /// Increase selected index by 1. Does nothing if nothing is currently selected.
    pub fn select_next(&mut self) {
        if let Some(i) = self.index() {
            self.set_index(i.saturating_add(1));
        };
    }

    /// Decrease selected index by 1. Does nothing if nothing is currently selected.
    pub fn select_previous(&mut self) {
        if let Some(i) = self.index() {
            self.set_index(i.saturating_sub(1));
        };
    }

    /// Select the first index. Does nothing if nothing is currently selected.
    pub fn select_first(&mut self) {
        if self.index.is_some() {
            self.set_index(0);
        }
    }

    /// Select the last index. Does nothing if nothing is currently selected.
    pub fn select_last(&mut self) {
        if self.index.is_some() {
            self.set_index(self.bound.saturating_sub(1))
        }
    }

    pub fn is_selected(&self, i: usize) -> bool {
        if let Some(selected) = self.index {
            return selected == i as u64;
        }
        false
    }
}

pub enum SelectionType {
    Row,
    Column,
    Cell,
    None,
}

#[derive(Clone)]
pub struct Selection {
    pub row: SelectionDimension,
    pub column: SelectionDimension,
}

impl Selection {
    pub fn default(row_bound: u64) -> Self {
        Selection {
            row: SelectionDimension {
                index: Some(0),
                bound: row_bound,
            },
            column: SelectionDimension {
                index: None,
                bound: 0,
            },
        }
    }

    pub fn selection_type(&self) -> SelectionType {
        if self.row.index.is_some() && self.column.index.is_some() {
            SelectionType::Cell
        } else if self.row.index.is_some() {
            SelectionType::Row
        } else if self.column.index.is_some() {
            SelectionType::Column
        } else {
            SelectionType::None
        }
    }
}

pub struct RowsView {
    reader: CsvLensReader,
    rows: Vec<Row>,
    num_rows: u64,
    rows_from: u64,
    filter: Option<RowsFilter>,
    columns_filter: Option<ColumnsFilter>,
    pub selection: Selection,
    elapsed: Option<u128>,
}

impl RowsView {
    pub fn new(mut reader: CsvLensReader, num_rows: u64) -> Result<RowsView> {
        let rows_from = 0;
        let rows = reader.get_rows(rows_from, num_rows)?;
        let view = Self {
            reader,
            rows,
            num_rows,
            rows_from,
            filter: None,
            columns_filter: None,
            selection: Selection::default(num_rows),
            elapsed: None,
        };
        Ok(view)
    }

    pub fn headers(&self) -> &Vec<String> {
        if let Some(columns_filter) = &self.columns_filter {
            columns_filter.filtered_headers()
        } else {
            &self.reader.headers
        }
    }

    pub fn rows(&self) -> &Vec<Row> {
        &self.rows
    }

    pub fn get_cell_value(&self, column_name: &str) -> Option<String> {
        if let (Some(column_index), Some(row_index)) = (
            self.headers().iter().position(|col| col == column_name),
            self.selection.row.index(),
        ) {
            return self
                .rows()
                .get(row_index as usize)
                .and_then(|row| row.fields.get(column_index))
                .cloned();
        }
        None
    }

    pub fn num_rows(&self) -> u64 {
        self.num_rows
    }

    pub fn set_num_rows(&mut self, num_rows: u64) -> Result<()> {
        if num_rows == self.num_rows {
            return Ok(());
        }
        self.num_rows = num_rows;
        self.do_get_rows()?;
        Ok(())
    }

    pub fn set_filter(&mut self, finder: &find::Finder) -> Result<()> {
        let filter = RowsFilter::new(finder, self.rows_from, self.num_rows);
        // only need to reload rows if the currently shown indices changed
        let mut needs_reload = true;
        if let Some(cur_filter) = &self.filter {
            if cur_filter.indices == filter.indices {
                needs_reload = false;
            }
        }
        // but always need to update filter because it holds other states such
        // as total count
        self.filter = Some(filter);
        if needs_reload {
            self.do_get_rows()
        } else {
            Ok(())
        }
    }

    pub fn is_filter(&self) -> bool {
        self.filter.is_some()
    }

    pub fn reset_filter(&mut self) -> Result<()> {
        if !self.is_filter() {
            return Ok(());
        }
        self.filter = None;
        self.do_get_rows()
    }

    pub fn columns_filter(&self) -> Option<&ColumnsFilter> {
        self.columns_filter.as_ref()
    }

    pub fn set_columns_filter(&mut self, target: Regex) -> Result<()> {
        self.columns_filter = Some(ColumnsFilter::new(target, &self.reader.headers));
        self.do_get_rows()
    }

    pub fn reset_columns_filter(&mut self) -> Result<()> {
        self.columns_filter = None;
        self.do_get_rows()
    }

    pub fn rows_from(&self) -> u64 {
        self.rows_from
    }

    pub fn set_rows_from(&mut self, rows_from_: u64) -> Result<()> {
        let rows_from = if let Some(n) = self.bottom_rows_from() {
            min(rows_from_, n)
        } else {
            rows_from_
        };
        if rows_from == self.rows_from {
            return Ok(());
        }
        self.rows_from = rows_from;
        self.do_get_rows()?;
        Ok(())
    }

    pub fn selected_offset(&self) -> Option<u64> {
        self.selection
            .row
            .index()
            .map(|x| x.saturating_add(self.rows_from))
    }

    pub fn elapsed(&self) -> Option<u128> {
        self.elapsed
    }

    pub fn get_total_line_numbers(&self) -> Option<usize> {
        self.reader.get_total_line_numbers()
    }

    pub fn get_total_line_numbers_approx(&self) -> Option<usize> {
        self.reader.get_total_line_numbers_approx()
    }

    pub fn in_view(&self, row_index: u64) -> bool {
        let last_row = self.rows_from().saturating_add(self.num_rows());
        if row_index >= self.rows_from() && row_index < last_row {
            return true;
        }
        false
    }

    pub fn handle_control(&mut self, control: &Control) -> Result<()> {
        match control {
            Control::ScrollDown => {
                if let Some(i) = self.selection.row.index() {
                    if i == self.num_rows - 1 {
                        self.increase_rows_from(1)?;
                    } else {
                        self.selection.row.select_next();
                    }
                } else {
                    self.increase_rows_from(1)?;
                }
            }
            Control::ScrollPageDown => {
                self.increase_rows_from(self.num_rows)?;
                self.selection.row.select_first()
            }
            Control::ScrollUp => {
                if let Some(i) = self.selection.row.index() {
                    if i == 0 {
                        self.decrease_rows_from(1)?;
                    } else {
                        self.selection.row.select_previous();
                    }
                } else {
                    self.decrease_rows_from(1)?;
                }
            }
            Control::ScrollPageUp => {
                self.decrease_rows_from(self.num_rows)?;
                self.selection.row.select_first()
            }
            Control::ScrollTop => {
                self.set_rows_from(0)?;
                self.selection.row.select_first()
            }
            Control::ScrollBottom => {
                if let Some(total) = self.get_total() {
                    let rows_from = total.saturating_sub(self.num_rows as usize) as u64;
                    self.set_rows_from(rows_from)?;
                }
                self.selection.row.select_last()
            }
            Control::ScrollTo(n) => {
                let mut rows_from = n.saturating_sub(1) as u64;
                if let Some(n) = self.bottom_rows_from() {
                    rows_from = min(rows_from, n);
                }
                self.set_rows_from(rows_from)?;
                self.selection.row.select_first()
            }
            _ => {}
        }
        Ok(())
    }

    fn get_total(&self) -> Option<usize> {
        if let Some(filter) = &self.filter {
            return Some(filter.total);
        } else if let Some(n) = self
            .reader
            .get_total_line_numbers()
            .or_else(|| self.reader.get_total_line_numbers_approx())
        {
            return Some(n);
        }
        None
    }

    fn increase_rows_from(&mut self, delta: u64) -> Result<()> {
        let new_rows_from = self.rows_from.saturating_add(delta);
        self.set_rows_from(new_rows_from)?;
        Ok(())
    }

    fn decrease_rows_from(&mut self, delta: u64) -> Result<()> {
        let new_rows_from = self.rows_from.saturating_sub(delta);
        self.set_rows_from(new_rows_from)?;
        Ok(())
    }

    fn bottom_rows_from(&self) -> Option<u64> {
        // fix type conversion craziness
        if let Some(n) = self.get_total() {
            return Some(n.saturating_sub(self.num_rows as usize) as u64);
        }
        None
    }

    fn subset_columns(rows: &Vec<Row>, indices: &[usize]) -> Vec<Row> {
        let mut out = vec![];
        for row in rows {
            out.push(row.subset(indices));
        }
        out
    }

    fn do_get_rows(&mut self) -> Result<()> {
        let start = Instant::now();
        let mut rows = if let Some(filter) = &self.filter {
            let indices = &filter.indices;
            self.reader.get_rows_for_indices(indices)?
        } else {
            self.reader.get_rows(self.rows_from, self.num_rows)?
        };
        let elapsed = start.elapsed().as_micros();
        if let Some(columns_filter) = &self.columns_filter {
            rows = Self::subset_columns(&rows, columns_filter.indices());
        }
        self.rows = rows;
        self.elapsed = Some(elapsed);
        // current selected might be out of range, reset it
        self.selection.row.set_bound(self.rows.len() as u64);
        if let Some(row) = self.rows().first() {
            self.selection.column.set_bound(row.fields.len() as u64);
        }
        Ok(())
    }
}
