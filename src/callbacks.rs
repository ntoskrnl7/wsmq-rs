#[macro_export]
macro_rules! define_set_callback {
    ($prop:ident, $type:ty) => {
        pub fn $prop(mut self, callback: $type) -> Self {
            self.$prop = Some(callback);
            self
        }
    };
}
