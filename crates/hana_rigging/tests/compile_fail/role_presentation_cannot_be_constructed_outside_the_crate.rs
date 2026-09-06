use hana_rigging::KernelRolePresentation;
use hana_rigging::RolePresentation;
use hana_rigging::RolePresentationView;

fn main() {
    let _: KernelRolePresentation = RolePresentation(RolePresentationView::Presenting);
}
