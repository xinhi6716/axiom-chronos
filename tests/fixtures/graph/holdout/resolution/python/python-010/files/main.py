def run(flag):
    if flag:
        from a import compute
    else:
        from b import compute
    compute()
