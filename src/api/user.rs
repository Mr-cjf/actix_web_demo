pub mod user_info {
    use actix_web::{get, web, HttpResponse};

    #[get("/userInfo/{id}")]
    pub async fn get_user_info(id: web::Path<String>) -> HttpResponse {
        let user_info_id = id.into_inner();
        HttpResponse::Ok().body(format!("Get userInfo: {}", user_info_id))
    }
}
