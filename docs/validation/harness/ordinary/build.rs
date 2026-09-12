fn main(){
 let rules=vec![chr_syntax::Rule::simplify("rewrite",vec![chr_syntax::c("p",vec![chr_syntax::v(0)])],chr_syntax::c("q",vec![chr_syntax::v(0)]).into())];
 let code=chr_compiled::generate::emit("rewrite",&rules).unwrap();
 let text=format!("#[allow(unused_imports,unused_variables,unused_mut)] mod generated {{ use chr_compiled::{{Core,Cursor,Candidate,Selection,Application,Work,Compiled,Frame}}; {code} }}");
 std::fs::write(std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("rewrite.rs"),text).unwrap();
}
