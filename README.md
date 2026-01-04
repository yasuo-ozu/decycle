```rust
#[decycle::decycle]
mod some_module {
    #[decycle]
    use path::to::Trait;
    
    impl Trait for SomeType {}
}
```
