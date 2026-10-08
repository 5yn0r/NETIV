//! Commandes « intelligence » : what-if, santé, documentation, génération, tuteur.

use netiv_analysis::{DocFormat, generate_docs, health, what_if};
use netiv_chaos::Difficulty;
use netiv_education::generate::{GenerateSpec, TOPICS, generate_challenge};
use netiv_education::tutor;
use netiv_topology::generate::{GenSpec, TopoKind, generate};

use super::*;

/// Options `--clé valeur` lues, et mots restants.
type Parsed<'a> = (Vec<(&'a str, &'a str)>, Vec<&'a str>);

/// Lit `--clé valeur` dans `args`. Retourne les options et les mots restants.
fn options<'a>(args: &[&'a str], keys: &[&str]) -> Result<Parsed<'a>, CliError> {
    let mut opts = Vec::new();
    let mut rest = Vec::new();
    let mut it = args.iter().copied();
    while let Some(w) = it.next() {
        if keys.contains(&w) {
            let v = it
                .next()
                .ok_or_else(|| CliError(format!("{w} attend une valeur")))?;
            opts.push((w, v));
        } else {
            rest.push(w);
        }
    }
    Ok((opts, rest))
}

fn opt<'a>(opts: &[(&'a str, &'a str)], key: &str) -> Option<&'a str> {
    opts.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
}

fn parse_usize(v: &str, what: &str) -> Result<usize, CliError> {
    v.parse()
        .map_err(|_| CliError(format!("{what} invalide : « {v} »")))
}

impl Session {
    pub(super) fn cmd_what_if(
        &mut self,
        args: &[&str],
        out: &mut dyn Write,
    ) -> Result<(), CliError> {
        if args.is_empty() {
            return err(
                "usage : what-if \"R1 goes down\"  (ou « block tcp/443 », « disconnect SW2 »…)",
            );
        }
        let text = args.join(" ").trim_matches('"').to_string();
        let impact = what_if(self.sim()?, &text).map_err(CliError)?;
        write!(out, "{}", impact.render())?;
        Ok(())
    }

    pub(super) fn cmd_health(&mut self, out: &mut dyn Write) -> Result<(), CliError> {
        write!(out, "{}", health(self.sim()?).render())?;
        Ok(())
    }

    pub(super) fn cmd_docs(&mut self, args: &[&str], out: &mut dyn Write) -> Result<(), CliError> {
        let (format, file) = match args {
            [f] => (*f, None),
            [f, file] => (*f, Some(*file)),
            _ => return err("usage : docs md|json|yaml [fichier]"),
        };
        let format: DocFormat = format.parse().map_err(CliError)?;
        let text = generate_docs(self.sim()?, &self.name, format).map_err(CliError)?;
        match file {
            None => write!(out, "{text}")?,
            Some(f) => {
                self.write_text(f, &text)?;
                writeln!(
                    out,
                    "Documentation écrite dans {f} ({} octets).",
                    text.len()
                )?;
            }
        }
        Ok(())
    }

    pub(super) fn cmd_topology(
        &mut self,
        args: &[&str],
        out: &mut dyn Write,
    ) -> Result<(), CliError> {
        let usage = "usage : topology generate --type <type> --routers N --hosts M [--seed S] [--save fichier.yaml]";
        let ["generate", rest @ ..] = args else {
            return err(usage);
        };
        let (opts, extra) = options(
            rest,
            &["--type", "--routers", "--hosts", "--seed", "--save"],
        )?;
        if !extra.is_empty() {
            return err(usage);
        }
        let kind: TopoKind = opt(&opts, "--type")
            .ok_or_else(|| CliError(usage.into()))?
            .parse()
            .map_err(CliError)?;
        let routers = opt(&opts, "--routers").map_or(Ok(kind.min_routers().max(3)), |v| {
            parse_usize(v, "nombre de routeurs")
        })?;
        let hosts =
            opt(&opts, "--hosts").map_or(Ok(10), |v| parse_usize(v, "nombre de machines"))?;
        let seed = opt(&opts, "--seed").map_or(Ok(1), |v| {
            v.parse::<u64>()
                .map_err(|_| CliError(format!("graine invalide : « {v} »")))
        })?;
        let def = generate(&GenSpec {
            kind,
            routers,
            hosts,
            seed,
        })
        .map_err(CliError)?;
        let yaml = serde_yaml_ng::to_string(&def).map_err(|e| CliError(e.to_string()))?;
        if let Some(file) = opt(&opts, "--save") {
            self.write_text(file, &yaml)?;
            writeln!(out, "Topologie écrite dans {file}.")?;
        }
        self.load_yaml(&yaml, out)
    }

    pub(super) fn challenge_generate(
        &mut self,
        args: &[&str],
        out: &mut dyn Write,
    ) -> Result<(), CliError> {
        let usage = format!(
            "usage : challenge generate --topic <{}> --difficulty facile|moyen|difficile [--seed N] [--save fichier.netiv]",
            TOPICS.join("|")
        );
        let (opts, extra) = options(args, &["--topic", "--difficulty", "--seed", "--save"])?;
        if !extra.is_empty() {
            return err(usage);
        }
        let topic = opt(&opts, "--topic").ok_or_else(|| CliError(usage.clone()))?;
        let difficulty = match opt(&opts, "--difficulty") {
            None => Difficulty::Medium,
            Some(d) => Difficulty::from_slug(d).ok_or_else(|| {
                CliError(format!(
                    "difficulté inconnue : « {d} » (facile, moyen, difficile)"
                ))
            })?,
        };
        let seed = opt(&opts, "--seed").map_or(Ok(1), |v| {
            v.parse::<u64>()
                .map_err(|_| CliError(format!("graine invalide : « {v} »")))
        })?;
        let challenge = generate_challenge(&GenerateSpec {
            topic,
            difficulty,
            seed,
        })
        .map_err(CliError)?;
        if let Some(file) = opt(&opts, "--save") {
            let bundle = challenge.to_bundle(&self.profile.name).map_err(CliError)?;
            self.write_text(file, &bundle.to_yaml()?)?;
            writeln!(
                out,
                "Challenge « {} » enregistré dans {file}.",
                challenge.id
            )?;
        }
        self.start_loaded(challenge, None, out)
    }

    pub(super) fn cmd_tutor(&mut self, args: &[&str], out: &mut dyn Write) -> Result<(), CliError> {
        let question = args.join(" ");
        let active = self.run.as_mut().filter(|r| !r.is_finished());
        let reply = tutor::ask(active, &question).map_err(CliError)?;
        writeln!(out, "{}", reply.text)?;
        if reply.cost_hint {
            let used = self.run.as_ref().map_or(0, |r| r.hints_used());
            writeln!(out, "(−10 points — indices utilisés : {used})")?;
        }
        Ok(())
    }
}
