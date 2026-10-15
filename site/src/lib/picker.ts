// The package picker's decision, from the data in src/data/packages.yaml (which the install
// guide's package table is also rendered from).

export interface PackageData {
  packages: {
    id: string;
    implementation: string;
    architectures: string[];
    glibc_min: string | null;
    binary_size: string;
    protocols: string[];
    features: string[];
  }[];
  features: Record<string, { label: string }>;
  protocols: Record<string, string>;
  /** Protocols tested against simulators only, not yet on real equipment. */
  preview_protocols?: string[];
  distributions: { id: string; label: string; glibc: string | null }[];
  rust_glibc_comfort: string;
}

export interface Answers {
  architecture: string;
  distribution: string;
  protocols: string[];
  features: string[];
  smallFootprint: boolean;
}

export interface Recommendation {
  pick: 'tedge-dot-rs' | 'tedge-dot-c';
  /** Why this package, one sentence each. */
  reasons: string[];
  /** What the recommended package will not do for these answers. */
  caveats: string[];
  /** Another way to get there, when there is one. */
  alternative?: string;
}

function versionLess(a: string, b: string): boolean {
  const pa = a.split('.').map(Number);
  const pb = b.split('.').map(Number);
  for (let i = 0; i < Math.max(pa.length, pb.length); i++) {
    const d = (pa[i] ?? 0) - (pb[i] ?? 0);
    if (d) return d < 0;
  }
  return false;
}

export function recommend(data: PackageData, input: Answers): Recommendation {
  // A serial PROFIBUS line is still PROFIBUS: it needs the protocol as well.
  const a = input.features.includes('profibus-serial') && !input.protocols.includes('profibus')
    ? { ...input, protocols: [...input.protocols, 'profibus'] }
    : input;
  const rs = data.packages.find((p) => p.id === 'tedge-dot-rs')!;
  const c = data.packages.find((p) => p.id === 'tedge-dot-c')!;
  const distro = data.distributions.find((d) => d.id === a.distribution);
  const profibus = a.protocols.includes('profibus');

  // What only the Rust build has, among the answers.
  const rsOnly = a.features.filter((f) => rs.features.includes(f) && !c.features.includes(f));
  const needRs = rsOnly.map((f) => `${data.features[f]?.label ?? f} is supported by tedge-dot-rs only.`);

  // What keeps the Rust build off this gateway.
  const rsBlocked: string[] = [];
  if (a.architecture && !rs.architectures.includes(a.architecture)) {
    rsBlocked.push(
      `tedge-dot-rs is built for ${rs.architectures.join(' and ')} only; tedge-dot-c is also built for ${c.architectures
        .filter((x) => !rs.architectures.includes(x))
        .join(', ')}.`,
    );
  }
  if (distro?.glibc && versionLess(distro.glibc, data.rust_glibc_comfort)) {
    rsBlocked.push(
      `${distro.label} ships glibc ${distro.glibc}. tedge-dot-c is built against a glibc ${c.glibc_min} floor, so it runs there; the Rust package is not tested on distributions that old.`,
    );
  }

  // The default package, and why it suits these answers in particular.
  const forC = [
    ...rsBlocked,
    ...(profibus ? ['PROFIBUS-DP ships in tedge-dot-c only: the Rust package is built without it.'] : []),
    ...(a.smallFootprint ? [`tedge-dot-c is about ${c.binary_size}, against ${rs.binary_size} for the Rust binary.`] : []),
  ];
  const sourceBuild =
    'Build tedge-dot-rs from source on Linux with `cargo build --manifest-path impl/rust/Cargo.toml --features profibus`.';

  if (!needRs.length) {
    return {
      pick: 'tedge-dot-c',
      reasons: [
        'It is the default: every protocol, a small binary, and packages down to a glibc 2.17 floor.',
        ...forC,
      ],
      caveats: [],
      alternative: profibus ? `Or use the Rust build: ${sourceBuild[0].toLowerCase()}${sourceBuild.slice(1)}` : undefined,
    };
  }
  if (rsBlocked.length) {
    // The Rust build is what the features need, but it does not run here.
    return {
      pick: 'tedge-dot-c',
      reasons: forC,
      caveats: needRs.map((r) => r.replace('is supported by tedge-dot-rs only.', 'is not available in tedge-dot-c.')),
    };
  }
  return {
    pick: 'tedge-dot-rs',
    reasons: [
      ...needRs,
      ...(profibus ? ['PROFIBUS-DP is not in the released Rust package, but the Rust source has it.'] : []),
    ],
    caveats: [],
    alternative: profibus ? sourceBuild : 'Without that, the default tedge-dot-c is the smaller choice.',
  };
}
