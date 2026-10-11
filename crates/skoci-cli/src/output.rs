use serde::Serialize;
use skoci_api::error::SkociError;
use skoci_api::models::ApiResponse;

pub fn print_result<T: Serialize>(json_mode: bool, res: Result<ApiResponse<T>, SkociError>) {
    match res {
        Ok(api_resp) => {
            if json_mode {
                let serialized = serde_json::to_string_pretty(&api_resp).unwrap();
                println!("{serialized}");
            } else {
                println!("{}", api_resp.summary);
                if let Some(data) = &api_resp.data {
                    let pretty = serde_json::to_string_pretty(data).unwrap();
                    println!("{pretty}");
                }
            }
        }
        Err(err) => {
            let api_err = ApiResponse::<()>::err(err.clone());
            if json_mode {
                let serialized = serde_json::to_string_pretty(&api_err).unwrap();
                eprintln!("{serialized}");
            } else {
                eprintln!("Error: {}", err.message);
                eprintln!("Solusi: {}", err.fix);
            }
            std::process::exit(1);
        }
    }
}
