/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! `UITableView`.
//!
//! Only enough for a game to create one, e.g. from a nib. It never asks its
//! data source for anything, so it shows no rows.

use crate::frameworks::core_graphics::{CGFloat, CGRect};
use crate::frameworks::foundation::NSInteger;
use crate::objc::{
    id, impl_HostObject_with_superclass, msg, nil, objc_classes, todo_objc_setter, ClassExports,
    NSZonePtr,
};

type UITableViewStyle = NSInteger;
const UITableViewStylePlain: UITableViewStyle = 0;
type UITableViewCellSeparatorStyle = NSInteger;

pub struct UITableViewHostObject {
    superclass: super::UIScrollViewHostObject,
    /// UITableViewDataSource, weak reference
    data_source: id,
    style: UITableViewStyle,
    row_height: CGFloat,
}
impl_HostObject_with_superclass!(UITableViewHostObject);
impl Default for UITableViewHostObject {
    fn default() -> Self {
        UITableViewHostObject {
            superclass: Default::default(),
            data_source: nil,
            style: UITableViewStylePlain,
            row_height: 44.0,
        }
    }
}

pub const CLASSES: ClassExports = objc_classes! {

(env, this, _cmd);

@implementation UITableView: UIScrollView

+ (id)allocWithZone:(NSZonePtr)_zone {
    let host_object = Box::<UITableViewHostObject>::default();
    env.objc.alloc_object(this, host_object, &mut env.mem)
}

- (id)dataSource {
    env.objc.borrow::<UITableViewHostObject>(this).data_source
}
- (())setDataSource:(id)data_source {
    env.objc.borrow_mut::<UITableViewHostObject>(this).data_source = data_source;
}

- (id)initWithFrame:(CGRect)frame style:(UITableViewStyle)style {
    let this: id = msg![env; this initWithFrame:frame];
    env.objc.borrow_mut::<UITableViewHostObject>(this).style = style;
    this
}
- (UITableViewStyle)style {
    env.objc.borrow::<UITableViewHostObject>(this).style
}

- (CGFloat)rowHeight {
    env.objc.borrow::<UITableViewHostObject>(this).row_height
}
- (())setRowHeight:(CGFloat)row_height {
    env.objc.borrow_mut::<UITableViewHostObject>(this).row_height = row_height;
}

// Nothing is drawn yet, so these do nothing.
- (())setSeparatorStyle:(UITableViewCellSeparatorStyle)style {
    todo_objc_setter!(this, style);
}
- (())setSeparatorColor:(id)color {
    todo_objc_setter!(this, color);
}
- (())setSectionHeaderHeight:(CGFloat)height {
    todo_objc_setter!(this, height);
}
- (())setSectionFooterHeight:(CGFloat)height {
    todo_objc_setter!(this, height);
}
- (())setAllowsSelection:(bool)allows {
    todo_objc_setter!(this, allows);
}
- (())setTableHeaderView:(id)view {
    todo_objc_setter!(this, view);
}
- (())setTableFooterView:(id)view {
    todo_objc_setter!(this, view);
}
- (())setBackgroundView:(id)view {
    todo_objc_setter!(this, view);
}
- (())reloadData {
    log!("TODO: [(UITableView*) {:?} reloadData]", this);
}
- (())deselectRowAtIndexPath:(id)_index_path animated:(bool)_animated {
    // No row can be selected.
}

@end

};
