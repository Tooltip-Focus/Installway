// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

#![allow(non_snake_case)] // tuple type parameters are also destructured below

//! Compatibility builders shared by the WinUI frontends.

use windows_reactor::*;

pub struct Element(View);
impl Element {
    pub fn from(value: impl Into<View>) -> Self {
        Self(value.into())
    }
    fn framed(self, f: impl FnOnce(Border) -> Border) -> Self {
        Self(f(Border::new()).content(self.0))
    }
    pub fn grid_row(self, v: i32) -> Self {
        self.framed(|b| b.grid_row(v))
    }
    pub fn grid_column(self, v: i32) -> Self {
        self.framed(|b| b.grid_column(v))
    }
    pub fn grid_column_span(self, v: i32) -> Self {
        self.framed(|b| b.grid_column_span(v))
    }
    pub fn margin(self, v: impl Into<Thickness>) -> Self {
        let v = v.into();
        self.framed(|b| b.margin(v))
    }
    pub fn width(self, v: f64) -> Self {
        self.framed(|b| b.width(v))
    }
    pub fn height(self, v: f64) -> Self {
        self.framed(|b| b.height(v))
    }
    pub fn vertical_alignment(self, v: VerticalAlignment) -> Self {
        self.framed(|b| b.vertical_alignment(v))
    }
    pub fn horizontal_alignment(self, v: HorizontalAlignment) -> Self {
        self.framed(|b| b.horizontal_alignment(v))
    }
}
impl From<Element> for View {
    fn from(v: Element) -> Self {
        v.0
    }
}

pub fn grid(children: Vec<Element>) -> GridCompat {
    GridCompat {
        children,
        rows: vec![],
        columns: vec![],
        padding: None,
        height: None,
    }
}
pub struct GridCompat {
    children: Vec<Element>,
    rows: Vec<GridLength>,
    columns: Vec<GridLength>,
    padding: Option<Thickness>,
    height: Option<f64>,
}
impl GridCompat {
    pub fn rows(mut self, v: impl IntoIterator<Item = GridLength>) -> Self {
        self.rows = v.into_iter().collect();
        self
    }
    pub fn columns(mut self, v: impl IntoIterator<Item = GridLength>) -> Self {
        self.columns = v.into_iter().collect();
        self
    }
    pub fn padding(mut self, v: impl Into<Thickness>) -> Self {
        self.padding = Some(v.into());
        self
    }
    pub fn height(mut self, v: f64) -> Self {
        self.height = Some(v);
        self
    }
}
impl From<GridCompat> for Element {
    fn from(v: GridCompat) -> Self {
        let mut g = Grid::new();
        if !v.rows.is_empty() {
            g = g.rows(v.rows)
        }
        if !v.columns.is_empty() {
            g = g.columns(v.columns)
        }
        let views = v.children.into_iter().map(View::from).enumerate();
        let inner = g.children((View::keyed_fragment(views),));
        let mut b = Border::new();
        if let Some(x) = v.padding {
            b = b.padding(x)
        }
        if let Some(x) = v.height {
            b = b.height(x)
        }
        Element::from(b.content(inner))
    }
}
impl From<GridCompat> for View {
    fn from(v: GridCompat) -> Self {
        View::from(<Element as From<GridCompat>>::from(v))
    }
}

pub trait CompatChildren {
    fn views(self) -> Vec<View>;
}
impl CompatChildren for () {
    fn views(self) -> Vec<View> {
        vec![]
    }
}
impl CompatChildren for Vec<Element> {
    fn views(self) -> Vec<View> {
        self.into_iter().map(View::from).collect()
    }
}
macro_rules! tuples{($($n:ident),+)=>{impl<$($n:Into<View>),+> CompatChildren for ($($n,)+){fn views(self)->Vec<View>{let($($n,)+)=self;vec![$($n.into(),)+]}}};}
tuples!(A);
tuples!(A, B);
tuples!(A, B, C);
tuples!(A, B, C, D);
pub fn vstack(c: impl CompatChildren) -> StackCompat {
    StackCompat::new(c.views(), Orientation::Vertical)
}
pub fn hstack(c: impl CompatChildren) -> StackCompat {
    StackCompat::new(c.views(), Orientation::Horizontal)
}
pub struct StackCompat {
    children: Vec<View>,
    orientation: Orientation,
    spacing: f64,
    padding: Option<Thickness>,
    width: Option<f64>,
    height: Option<f64>,
    horizontal: Option<HorizontalAlignment>,
    vertical: Option<VerticalAlignment>,
    column: Option<i32>,
}
impl StackCompat {
    fn new(children: Vec<View>, orientation: Orientation) -> Self {
        Self {
            children,
            orientation,
            spacing: 0.0,
            padding: None,
            width: None,
            height: None,
            horizontal: None,
            vertical: None,
            column: None,
        }
    }
    pub fn spacing(mut self, v: f64) -> Self {
        self.spacing = v;
        self
    }
    pub fn padding(mut self, v: impl Into<Thickness>) -> Self {
        self.padding = Some(v.into());
        self
    }
    pub fn width(mut self, v: f64) -> Self {
        self.width = Some(v);
        self
    }
    pub fn height(mut self, v: f64) -> Self {
        self.height = Some(v);
        self
    }
    pub fn horizontal_alignment(mut self, v: HorizontalAlignment) -> Self {
        self.horizontal = Some(v);
        self
    }
    pub fn vertical_alignment(mut self, v: VerticalAlignment) -> Self {
        self.vertical = Some(v);
        self
    }
    pub fn grid_column(mut self, v: i32) -> Self {
        self.column = Some(v);
        self
    }
}
impl From<StackCompat> for Element {
    fn from(v: StackCompat) -> Self {
        let inner = StackPanel::new()
            .orientation(v.orientation)
            .spacing(v.spacing)
            .children((View::keyed_fragment(v.children.into_iter().enumerate()),));
        let mut b = Border::new();
        if let Some(x) = v.padding {
            b = b.padding(x)
        }
        if let Some(x) = v.width {
            b = b.width(x)
        }
        if let Some(x) = v.height {
            b = b.height(x)
        }
        if let Some(x) = v.horizontal {
            b = b.horizontal_alignment(x)
        }
        if let Some(x) = v.vertical {
            b = b.vertical_alignment(x)
        }
        if let Some(x) = v.column {
            b = b.grid_column(x)
        }
        Element::from(b.content(inner))
    }
}
impl From<StackCompat> for View {
    fn from(v: StackCompat) -> Self {
        View::from(<Element as From<StackCompat>>::from(v))
    }
}

pub fn text_block(t: impl Into<String>) -> TextCompat {
    TextCompat {
        inner: TextBlock::new().text(t),
        padding: None,
    }
}
pub struct TextCompat {
    inner: TextBlock,
    padding: Option<Thickness>,
}
impl TextCompat {
    pub fn font_size(mut self, v: f64) -> Self {
        self.inner = self.inner.font_size(v);
        self
    }
    pub fn semibold(mut self) -> Self {
        self.inner = self.inner.font_weight(FontWeight::SEMI_BOLD);
        self
    }
    pub fn wrap(mut self) -> Self {
        self.inner = self.inner.text_wrapping(TextWrapping::Wrap);
        self
    }
    pub fn selectable(mut self) -> Self {
        self.inner = self.inner.is_text_selection_enabled(true);
        self
    }
    pub fn foreground(mut self, v: impl Into<Brush>) -> Self {
        self.inner = self.inner.foreground(v);
        self
    }
    pub fn height(mut self, v: f64) -> Self {
        self.inner = self.inner.height(v);
        self
    }
    pub fn padding(mut self, v: impl Into<Thickness>) -> Self {
        self.padding = Some(v.into());
        self
    }
}
impl From<TextCompat> for Element {
    fn from(v: TextCompat) -> Self {
        let inner: View = v.inner.into();
        if let Some(x) = v.padding {
            Element::from(Border::new().padding(x).content(inner))
        } else {
            Element::from(inner)
        }
    }
}
impl From<TextCompat> for View {
    fn from(v: TextCompat) -> Self {
        View::from(<Element as From<TextCompat>>::from(v))
    }
}

pub fn button(t: impl Into<String>) -> ButtonCompat {
    ButtonCompat {
        inner: Button::new(),
        label: t.into(),
    }
}
pub struct ButtonCompat {
    inner: Button,
    label: String,
}
impl ButtonCompat {
    pub fn accent(mut self) -> Self {
        self.inner = self.inner.style(ButtonStyle::Accent);
        self
    }
    pub fn enabled(mut self, v: bool) -> Self {
        self.inner = self.inner.is_enabled(v);
        self
    }
    pub fn on_click(mut self, v: impl IntoUnitCallback) -> Self {
        self.inner = self.inner.on_click(v);
        self
    }
    pub fn width(mut self, v: f64) -> Self {
        self.inner = self.inner.width(v);
        self
    }
    pub fn margin(mut self, v: impl Into<Thickness>) -> Self {
        self.inner = self.inner.margin(v);
        self
    }
    pub fn horizontal_alignment(mut self, v: HorizontalAlignment) -> Self {
        self.inner = self.inner.horizontal_alignment(v);
        self
    }
    pub fn grid_column(mut self, v: i32) -> Self {
        self.inner = self.inner.grid_column(v);
        self
    }
    pub fn with_key(self, _: impl Into<Key>) -> Self {
        self
    }
}
impl From<ButtonCompat> for Element {
    fn from(v: ButtonCompat) -> Self {
        Element::from(v.inner.content(v.label))
    }
}
impl From<ButtonCompat> for View {
    fn from(v: ButtonCompat) -> Self {
        View::from(<Element as From<ButtonCompat>>::from(v))
    }
}

pub fn scroll_viewer(c: impl Into<View>) -> ScrollCompat {
    ScrollCompat {
        child: c.into(),
        vertical: None,
    }
}
pub struct ScrollCompat {
    child: View,
    vertical: Option<ScrollBarVisibility>,
}
impl ScrollCompat {
    pub fn vertical_scroll_bar_visibility(mut self, v: ScrollBarVisibility) -> Self {
        self.vertical = Some(v);
        self
    }
}
impl From<ScrollCompat> for View {
    fn from(v: ScrollCompat) -> Self {
        let mut s = ScrollViewer::new();
        if let Some(x) = v.vertical {
            s = s.vertical_scroll_bar_visibility(x)
        }
        s.content(v.child)
    }
}
impl From<ScrollCompat> for Element {
    fn from(v: ScrollCompat) -> Self {
        Element::from(View::from(v))
    }
}

pub fn text_box(v: impl Into<String>) -> TextBox {
    TextBox::new().text(v)
}
pub fn check_box(v: bool) -> CheckCompat {
    CheckCompat {
        inner: CheckBox::new().is_checked(v),
        label: String::new(),
    }
}
pub struct CheckCompat {
    inner: CheckBox,
    label: String,
}
impl CheckCompat {
    pub fn content(mut self, v: impl Into<String>) -> Self {
        self.label = v.into();
        self
    }
    pub fn on_checked(mut self, v: impl IntoPayloadCallback<bool>) -> Self {
        self.inner = self.inner.on_is_checked_changed(v);
        self
    }
    pub fn margin(mut self, v: impl Into<Thickness>) -> Self {
        self.inner = self.inner.margin(v);
        self
    }
}
impl From<CheckCompat> for Element {
    fn from(v: CheckCompat) -> Self {
        Element::from(v.inner.content(v.label))
    }
}
impl From<CheckCompat> for View {
    fn from(v: CheckCompat) -> Self {
        View::from(<Element as From<CheckCompat>>::from(v))
    }
}
