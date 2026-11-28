/** @type {import('next').NextConfig} */
const nextConfig = {
  reactStrictMode: true,
  async redirects() {
    return [
      // The home page moved to https://deepflux.space/#remote (owner decision
      // 2026-11-28: one site to promote instead of three). Only "/" goes.
      // Every route the installed apps depend on stays — /api/*, /downloads/*,
      // /verify, /account — and so does /reference. Temporary on purpose, so
      // browsers do not cache it; make it permanent once the layout is settled.
      {
        source: '/',
        destination: 'https://deepflux.space/#remote',
        permanent: false,
      },
    ];
  },
};

export default nextConfig;
