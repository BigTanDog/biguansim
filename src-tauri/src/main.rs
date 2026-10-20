// 发布构建不带控制台窗口
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    biguansim_lib::run()
}
