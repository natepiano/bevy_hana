macro_rules! impl_binding_forwards {
    (
        $bindings:ty,
        $set:ty,
        $entry:ty,
        entries($($entries_vis:tt)*),
        enabled_entries($($enabled_entries_vis:tt)*)
    ) => {
        impl $bindings {
            /// Returns the number of bindings.
            #[must_use]
            pub const fn len(&self) -> usize {
                let set: &$set = &self.0;
                set.len()
            }

            /// Returns `true` when there are no bindings.
            #[must_use]
            pub const fn is_empty(&self) -> bool {
                let set: &$set = &self.0;
                set.is_empty()
            }

            /// Returns binding entries.
            #[must_use]
            $($entries_vis)* fn entries(&self) -> &[$entry] {
                let set: &$set = &self.0;
                set.entries()
            }

            /// Returns binding entries that participate in runtime input.
            $($enabled_entries_vis)* fn enabled_entries(&self) -> impl Iterator<Item = &$entry> {
                let set: &$set = &self.0;
                set.enabled_entries()
            }
        }
    };
}
