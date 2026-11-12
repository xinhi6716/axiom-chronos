def rolling_mean(values):
    present = [v for v in values if v is not None]
    return sum(present) / len(present) if present else None
