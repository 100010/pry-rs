use pry::pry;

#[derive(Debug)]
#[allow(dead_code)]
struct User {
    name: String,
    age: u32,
    tags: Vec<String>,
}

fn main() {
    let user = User {
        name: "alice".to_string(),
        age: 30,
        tags: vec!["admin".to_string(), "beta".to_string()],
    };
    let scores = [88, 92, 75];
    let total: i32 = scores.iter().sum();

    // Execution pauses here. Try: ls, p user, scores, total, whereami, bt, c
    pry!(user, scores, total, user.tags.len());

    println!("resumed! total = {total}");
}
