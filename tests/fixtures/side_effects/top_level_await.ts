// Should detect: top-level await
const db = await connectDatabase();
await initSchema();

export { db };
