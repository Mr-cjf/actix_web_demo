pub mod agency {
    use actix_web::{
        connect, delete, get, head, options, patch, post, put, trace, web, HttpResponse,
    };

    #[get("/agency/{id}")]
    pub async fn get_agency(id: web::Path<String>) -> HttpResponse {
        let agency_id = id.into_inner();
        HttpResponse::Ok().body(format!("Get agency: {}", agency_id))
    }

    #[post("/agency")]
    pub async fn create_agency() -> HttpResponse {
        HttpResponse::Ok().body("agency created")
    }

    #[put("/agency/{id}")]
    pub async fn update_agency(id: web::Path<String>) -> HttpResponse {
        HttpResponse::Ok().body(format!("Updated agency: {}", id))
    }

    #[delete("/agency/{id}")]
    pub async fn delete_agency(id: web::Path<String>) -> HttpResponse {
        HttpResponse::Ok().body(format!("Deleted agency: {}", id))
    }

    #[head("/head")]
    pub async fn head_example() -> HttpResponse {
        HttpResponse::Ok().body("HEAD request received")
    }

    #[connect("/connect")]
    pub async fn connect_example() -> HttpResponse {
        HttpResponse::Ok().body("CONNECT request received")
    }

    #[options("/options")]
    pub async fn options_example() -> HttpResponse {
        HttpResponse::Ok().body("OPTIONS request received")
    }

    #[trace("/trace")]
    pub async fn trace_example() -> HttpResponse {
        HttpResponse::Ok().body("TRACE request received")
    }

    #[patch("/patch")]
    pub async fn patch_example() -> HttpResponse {
        HttpResponse::Ok().body("PATCH request received")
    }
}
