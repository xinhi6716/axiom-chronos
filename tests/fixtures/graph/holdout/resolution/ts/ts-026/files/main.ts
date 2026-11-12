function run(registry: Record<string, () => void>, key: string) {
  registry[key]();
}
