def run(flag):
    if flag:
        from a import measure
    else:
        from b import measure
    measure()
