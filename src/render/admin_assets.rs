//! The admin UI (templates + stylesheet) is baked into the binary rather
//! than scanned from `./templates` / `./static` like the site theme. It is
//! not written out by `init`/`theme` and not user-overridable: upgrading the
//! binary upgrades the admin UI, with nothing for a theme to break.

use rust_embed::Embed;

#[derive(Embed)]
#[folder = "admin_templates/"]
struct AdminTemplates;

#[derive(Embed)]
#[folder = "admin_static/"]
struct AdminStatic;

/// Registers every `admin_templates/*.html` file under the `admin/` prefix
/// (e.g. `admin_templates/base.html` -> `admin/base.html`), matching the
/// names admin `TemplateCtx` impls and `{% extends %}` tags already use.
pub fn register(env: &mut minijinja::Environment<'static>) -> anyhow::Result<()> {
    for name in AdminTemplates::iter() {
        let file = AdminTemplates::get(&name).expect("just listed by iter()");
        let source = std::str::from_utf8(&file.data)?.to_string();
        env.add_template_owned(format!("admin/{name}"), source)?;
    }
    Ok(())
}

/// Raw bytes of the baked-in admin stylesheet, served at `/admin/admin.css`.
pub fn css() -> std::borrow::Cow<'static, [u8]> {
    AdminStatic::get("admin.css").expect("admin_static/admin.css is embedded at compile time").data
}
