use regex::bytes::{CaptureLocations, Regex, RegexBuilder};

pub const MAX_RULES: usize = 16;
pub const MAX_VALUE: usize = 253;
const MAX_PATTERN: usize = 2048;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    InvalidPlan,
    DuplicateAttribute,
    RequiredOutputMissing,
    InvalidOutput,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot<'a> {
    Missing,
    Value(&'a [u8]),
    Duplicate,
}

pub struct Received<'a, const N: usize> {
    slots: [Slot<'a>; N],
}

impl<'a, const N: usize> Received<'a, N> {
    pub fn new() -> Self {
        Self {
            slots: [Slot::Missing; N],
        }
    }

    pub fn record(&mut self, slot: usize, bytes: &'a [u8]) -> Result<(), Error> {
        let destination = self.slots.get_mut(slot).ok_or(Error::InvalidPlan)?;
        *destination = if *destination == Slot::Missing {
            Slot::Value(bytes)
        } else {
            Slot::Duplicate
        };
        Ok(())
    }

    fn read(&self, slot: usize) -> Result<Option<&'a [u8]>, Error> {
        match self.slots.get(slot).ok_or(Error::InvalidPlan)? {
            Slot::Missing => Ok(None),
            Slot::Value(value) if value.len() <= MAX_VALUE => Ok(Some(value)),
            Slot::Value(_) => Err(Error::InvalidOutput),
            Slot::Duplicate => Err(Error::DuplicateAttribute),
        }
    }
}

impl<const N: usize> Default for Received<'_, N> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy)]
pub enum Transform {
    Keep,
    LowerAscii,
    UpperAscii,
    HexDecode,
    HexUpper,
}

pub struct RuleSpec<'a> {
    pub guard: Option<(usize, &'a str)>,
    pub pattern: &'a str,
    pub capture: usize,
    pub prefix: &'a [u8],
    pub suffix: &'a [u8],
    pub transform: Transform,
}

#[derive(Clone)]
struct Rule {
    guard: Option<(usize, Regex)>,
    pattern: Regex,
    capture: usize,
    prefix: Vec<u8>,
    suffix: Vec<u8>,
    transform: Transform,
}

pub struct TargetSpec<'a> {
    pub slot: usize,
    pub required: bool,
    pub min_octets: usize,
    pub max_octets: usize,
    pub rules: &'a [RuleSpec<'a>],
}

#[derive(Clone)]
struct Target {
    slot: usize,
    required: bool,
    min_octets: usize,
    max_octets: usize,
    rules: Vec<Rule>,
}

pub struct RewritePlan<const N: usize> {
    targets: Vec<Target>,
}

struct Scratch<const N: usize> {
    values: [Vec<u8>; N],
    present: [bool; N],
}

impl<const N: usize> Default for Scratch<N> {
    fn default() -> Self {
        Self {
            values: std::array::from_fn(|_| Vec::new()),
            present: [false; N],
        }
    }
}

pub struct RewriteWorker<const N: usize> {
    targets: Vec<Target>,
    captures: Vec<Vec<CaptureLocations>>,
    scratch: Scratch<N>,
}

pub struct Working<'a, 'packet, const N: usize> {
    received: &'a Received<'packet, N>,
    scratch: &'a Scratch<N>,
}

impl<const N: usize> Working<'_, '_, N> {
    pub fn attribute(&self, slot: usize) -> Result<Option<&[u8]>, Error> {
        if *self.scratch.present.get(slot).ok_or(Error::InvalidPlan)? {
            return Ok(Some(&self.scratch.values[slot]));
        }
        self.received.read(slot)
    }
}

fn compile_regex(pattern: &str, whole_value: bool) -> Result<Regex, Error> {
    if pattern.len() > MAX_PATTERN {
        return Err(Error::InvalidPlan);
    }
    let expression = if whole_value {
        format!(r"\A(?:{pattern})\z")
    } else {
        pattern.to_owned()
    };
    RegexBuilder::new(&expression)
        .unicode(false)
        .size_limit(128 * 1024)
        .dfa_size_limit(128 * 1024)
        .build()
        .map_err(|_| Error::InvalidPlan)
}

impl<const N: usize> RewritePlan<N> {
    pub fn compile(specs: &[TargetSpec<'_>]) -> Result<Self, Error> {
        let mut seen = [false; N];
        let mut total = 0;
        let mut targets = Vec::with_capacity(specs.len().min(N));
        for spec in specs {
            let seen_slot = seen.get_mut(spec.slot).ok_or(Error::InvalidPlan)?;
            total += spec.rules.len();
            if *seen_slot
                || spec.min_octets > spec.max_octets
                || spec.max_octets > MAX_VALUE
                || spec.rules.is_empty()
                || total > MAX_RULES
                || (spec.required && spec.min_octets == 0)
            {
                return Err(Error::InvalidPlan);
            }
            *seen_slot = true;
            let mut rules = Vec::with_capacity(spec.rules.len());
            for rule in spec.rules {
                let guard = match rule.guard {
                    Some((slot, pattern)) if slot < N => {
                        Some((slot, compile_regex(pattern, false)?))
                    }
                    Some(_) => return Err(Error::InvalidPlan),
                    None => None,
                };
                let pattern = compile_regex(rule.pattern, true)?;
                if rule.capture >= pattern.captures_len()
                    || rule.prefix.len().saturating_add(rule.suffix.len()) > spec.max_octets
                {
                    return Err(Error::InvalidPlan);
                }
                rules.push(Rule {
                    guard,
                    pattern,
                    capture: rule.capture,
                    prefix: rule.prefix.to_vec(),
                    suffix: rule.suffix.to_vec(),
                    transform: rule.transform,
                });
            }
            targets.push(Target {
                slot: spec.slot,
                required: spec.required,
                min_octets: spec.min_octets,
                max_octets: spec.max_octets,
                rules,
            });
        }
        Ok(Self { targets })
    }

    pub fn worker(&self) -> RewriteWorker<N> {
        let targets = self.targets.clone();
        let captures = targets
            .iter()
            .map(|target| {
                target
                    .rules
                    .iter()
                    .map(|rule| rule.pattern.capture_locations())
                    .collect()
            })
            .collect();
        RewriteWorker {
            targets,
            captures,
            scratch: Scratch::default(),
        }
    }
}

impl<const N: usize> RewriteWorker<N> {
    pub fn then_lookup<T>(
        &mut self,
        received: &Received<'_, N>,
        lookup: impl FnOnce(Working<'_, '_, N>) -> T,
    ) -> Result<T, Error> {
        let scratch = &mut self.scratch;
        scratch.present.fill(false);
        for (target, captures) in self.targets.iter().zip(&mut self.captures) {
            if let Some(input) = received.read(target.slot)? {
                for (rule, locations) in target.rules.iter().zip(captures) {
                    if let Some((slot, guard)) = &rule.guard {
                        if !received
                            .read(*slot)?
                            .is_some_and(|value| guard.is_match(value))
                        {
                            continue;
                        }
                    }
                    if rule.pattern.captures_read(locations, input).is_none() {
                        continue;
                    }
                    let (start, end) = locations.get(rule.capture).ok_or(Error::InvalidOutput)?;
                    let value = &input[start..end];
                    let output = &mut scratch.values[target.slot];
                    output.clear();
                    output.extend_from_slice(&rule.prefix);
                    match rule.transform {
                        Transform::Keep => output.extend_from_slice(value),
                        Transform::LowerAscii => {
                            output.extend(value.iter().map(u8::to_ascii_lowercase))
                        }
                        Transform::UpperAscii => {
                            output.extend(value.iter().map(u8::to_ascii_uppercase))
                        }
                        Transform::HexDecode => {
                            let text =
                                std::str::from_utf8(value).map_err(|_| Error::InvalidOutput)?;
                            output.extend(
                                crate::option82::decode_hex(text)
                                    .map_err(|_| Error::InvalidOutput)?,
                            );
                        }
                        Transform::HexUpper => {
                            let text =
                                std::str::from_utf8(value).map_err(|_| Error::InvalidOutput)?;
                            crate::option82::decode_hex(text).map_err(|_| Error::InvalidOutput)?;
                            output.extend(value.iter().map(u8::to_ascii_uppercase));
                        }
                    }
                    output.extend_from_slice(&rule.suffix);
                    if !(target.min_octets..=target.max_octets).contains(&output.len()) {
                        return Err(Error::InvalidOutput);
                    }
                    scratch.present[target.slot] = true;
                    break;
                }
            }
            if target.required && !scratch.present[target.slot] {
                return Err(Error::RequiredOutputMissing);
            }
        }
        Ok(lookup(Working { received, scratch }))
    }
}
