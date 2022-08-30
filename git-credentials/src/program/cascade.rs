use crate::program::Cascade;
use crate::protocol::Context;
use crate::{helper, protocol, Program};

impl Default for Cascade {
    fn default() -> Self {
        Cascade {
            programs: Vec::new(),
            stderr: true,
        }
    }
}

/// Initialization
impl Cascade {
    /// Return the programs to run for the current platform.
    ///
    /// These are typically used as basis for all credential cascade invocations, with configured programs following afterwards.
    ///
    /// # Note
    ///
    /// These defaults emulate what typical git installations may use these days, as in fact it's a configurable which comes
    /// from installation-specific configuration files which we cannot know (or guess at best).
    /// This seems like an acceptable trade-off as helpers are ignored if they fail or are not existing.
    pub fn platform_builtin() -> Vec<Program> {
        if cfg!(target_os = "macos") {
            Some("osxkeychain")
        } else if cfg!(target_os = "linux") {
            Some("libsecret")
        } else if cfg!(target_os = "windows") {
            Some("manager-core")
        } else {
            None
        }
        .map(|name| vec![Program::from_custom_definition(name)])
        .unwrap_or_default()
    }
}

/// Builder
impl Cascade {
    /// Extend the list of programs to run `programs`.
    pub fn extend(mut self, programs: impl IntoIterator<Item = Program>) -> Self {
        self.programs.extend(programs);
        self
    }
}

/// Finalize
impl Cascade {
    /// Invoke the cascade by `invoking` each program with `action`, and configuring potential prompts with `prompt` options.
    /// The latter can also be used to disable the prompt entirely when setting the `mode` to [`Disable`][git_prompt::Mode::Disable];=.
    ///
    /// When _getting_ credentials, all programs are asked until the credentials are complete, stopping the cascade.
    /// When _storing_ or _erasing_ all programs are instructed in order.
    pub fn invoke(&mut self, mut action: helper::Action, mut prompt: git_prompt::Options<'_>) -> protocol::Result {
        action.context_mut().map(Context::destructure_url).transpose()?;

        for program in &mut self.programs {
            program.stderr = self.stderr;
            match helper::invoke::raw(program, &action) {
                Ok(None) => {}
                Ok(Some(stdout)) => {
                    let ctx = Context::from_bytes(&stdout)?;
                    if let Some(dst_ctx) = action.context_mut() {
                        if let Some(src) = ctx.path {
                            dst_ctx.path = Some(src);
                        }
                        for (src, dst) in [
                            (ctx.protocol, &mut dst_ctx.protocol),
                            (ctx.host, &mut dst_ctx.host),
                            (ctx.username, &mut dst_ctx.username),
                            (ctx.password, &mut dst_ctx.password),
                        ] {
                            if let Some(src) = src {
                                *dst = Some(src);
                            }
                        }
                        if let Some(src) = ctx.url {
                            dst_ctx.url = Some(src);
                            dst_ctx.destructure_url()?;
                        }
                        if dst_ctx.username.is_some() && dst_ctx.password.is_some() {
                            break;
                        }
                        if ctx.quit.unwrap_or_default() {
                            dst_ctx.quit = ctx.quit;
                            break;
                        }
                    }
                }
                Err(helper::Error::CredentialsHelperFailed { .. }) => continue, // ignore helpers that we can't call
                Err(err) if action.context().is_some() => return Err(err.into()), // communication errors are fatal when getting credentials
                Err(_) => {} // for other actions, ignore everything, try the operation
            }
        }

        if prompt.mode != git_prompt::Mode::Disable {
            if let Some(ctx) = action.context_mut() {
                if ctx.username.is_none() {
                    let message = ctx.to_prompt("Username");
                    prompt.mode = git_prompt::Mode::Visible;
                    ctx.username = git_prompt::ask(&message, &prompt)
                        .map_err(|err| protocol::Error::Prompt {
                            prompt: message,
                            source: err,
                        })?
                        .into();
                }
                if ctx.password.is_none() {
                    let message = ctx.to_prompt("Password");
                    prompt.mode = git_prompt::Mode::Hidden;
                    ctx.password = git_prompt::ask(&message, &prompt)
                        .map_err(|err| protocol::Error::Prompt {
                            prompt: message,
                            source: err,
                        })?
                        .into();
                }
            }
        }

        protocol::helper_outcome_to_result(
            action.context().map(|ctx| helper::Outcome {
                username: ctx.username.clone(),
                password: ctx.password.clone(),
                quit: ctx.quit.unwrap_or(false),
                next: ctx.to_owned().into(),
            }),
            action,
        )
    }
}
