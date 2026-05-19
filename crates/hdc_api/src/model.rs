use std::sync::Arc;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default)]
pub struct Vector {
    pub x: f64,
    pub y: f64,
}

impl Vector {
    pub fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

/*
"attributes": {
  "accessibilityId": "",
  "backgroundColor": "",
  "backgroundImage": "",
  "blur": "",
  "bounds": "[0,0][1084,2412]",
  "checkable": "",
  "checked": "",
  "clickable": "",
  "clip": "",
  "description": "",
  "displayId": "",
  "enabled": "",
  "focused": "",
  "hitTestBehavior": "",
  "hostWindowId": "",
  "id": "",
  "key": "",
  "longClickable": "",
  "opacity": "",
  "origBounds": "",
  "originalText": "",
  "scrollable": "",
  "selected": "",
  "text": "",
  "type": "",
  "zIndex": ""
}, */

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct LayoutAttribute {
    text: Option<String>,
    id: Option<String>,
    bounds: Option<String>,
    key: Option<String>,
    #[serde(rename = "type")]
    attribute_type: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Layout {
    pub attributes: Option<LayoutAttribute>,
    pub children: Option<Vec<Layout>>,
}

impl Layout {
    pub fn parse_to_finder(&self) -> LayoutFinder {
        LayoutFinder {
            left: None,
            right: Some(Arc::new(self.clone())),
            index: 0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct LayoutFinder {
    pub left: Option<Arc<Layout>>,
    pub right: Option<Arc<Layout>>,
    pub index: usize,
}
