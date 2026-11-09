export const qualityOptions = [
  { value: "best", label: "最佳可用画质" },
  { value: "2160", label: "最高 4K · 2160p" },
  { value: "1440", label: "最高 2K · 1440p" },
  { value: "1080", label: "最高 1080p" },
  { value: "720", label: "最高 720p" },
  { value: "480", label: "最高 480p" },
];

export const qualityName = (value: string) =>
  qualityOptions.find((q) => q.value === value)?.label ?? value;
