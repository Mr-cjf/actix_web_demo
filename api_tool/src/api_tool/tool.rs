pub mod tool_info {
    use actix_web::{get, web, HttpResponse};

    #[get("/toolInfo/{id}")]
    pub async fn get_tool_info(id: web::Path<String>) -> HttpResponse {
        let tool_info_id = id.into_inner();
        HttpResponse::Ok().body(format!("Get toolInfo: {}", tool_info_id))
    }
}
