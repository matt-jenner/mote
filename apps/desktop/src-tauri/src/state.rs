use std::sync::Mutex;

use photo_app_service::AppService;

pub struct DesktopState {
    pub service: Mutex<AppService>,
}
