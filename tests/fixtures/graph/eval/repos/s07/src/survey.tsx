export function scoreAnswers(answers: Array<number | null>): number {
  return answers.filter((answer): answer is number => answer !== null).reduce((a, b) => a + b, 0);
}
