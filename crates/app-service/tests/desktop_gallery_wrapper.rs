use photo_app_service::{AppConfig, AppService, GalleryScope, WallQueryRequest, WallUpdate};

#[tokio::test]
async fn desktop_start_scan_uses_the_selection_runtime_wrapper() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    std::fs::create_dir_all(source.join("child")).unwrap();
    let image = image::ImageBuffer::from_pixel(3, 2, image::Rgb([220_u8, 180_u8, 80_u8]));
    image.save(source.join("photo.jpg")).unwrap();
    image.save(source.join("child/photo.jpg")).unwrap();
    let config = AppConfig::new(temp.path().join("data"), temp.path().join("cache"));
    let service = AppService::open(config).unwrap();
    let mut updates = service.subscribe_wall_updates();

    let bootstrap = service.start_scan(&source).await.unwrap();

    assert!(bootstrap.active_source.is_some());
    assert_eq!(service.runtime_count_for_test(), 1);

    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if matches!(updates.recv().await, Ok(WallUpdate::MetadataSettled { .. })) {
                break;
            }
        }
    })
    .await
    .expect("desktop wrapper must forward the shared runtime settlement");

    let recursive = service
        .query_wall(WallQueryRequest::oldest_first())
        .await
        .unwrap();
    assert_eq!(recursive.items.len(), 2);
    service
        .update_gallery_scope(GalleryScope::CurrentFolder)
        .await
        .unwrap();
    let current = service
        .query_wall(WallQueryRequest::oldest_first())
        .await
        .unwrap();
    assert_eq!(current.items.len(), 1);
}
