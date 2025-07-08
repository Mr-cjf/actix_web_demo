pub mod agency_api {
    use actix_web::{get, web, HttpResponse};

    #[get("/agency/{id}")]
    pub async fn get_agency(id: web::Path<String>) -> HttpResponse {
        let agency_id = id.into_inner();
        HttpResponse::Ok().body(format!("Get agency: {}", agency_id))
    }
}
